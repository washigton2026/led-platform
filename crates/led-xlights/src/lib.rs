//! xLights project importer — reads an xLights show folder and produces a LUMYX
//! layout, **refusing silently-broken configs** instead of rendering them.
//!
//! What it reads:
//! - `xlights_networks.xml` — controllers (name, IP, protocol) and their universes.
//! - `xlights_rgbeffects.xml` — models (strips), start channels, node counts, groups.
//!
//! What it produces:
//! - [`ImportReport`] — models, controllers, total pixels, and **channel conflicts**.
//! - `Vec<PixelPhysical>` via [`ImportReport::assignments`] — feed it straight to
//!   `led_core::CompiledLayout::compile`.
//!
//! ## Invariants (lumyx-system-architect §1)
//! - Import happens **once**, at layout time — never on the frame hot path.
//! - Channel conflicts are a **gate**, not a warning: `assignments()` returns
//!   `Err` while conflicts exist. The user's real show folder had 35 overlapping
//!   StartChannel groups that xLights rendered anyway; LUMYX refuses.
//! - The parser is a minimal, std-only XML reader for machine-generated files.
//!   It handles elements, attributes, self-closing tags, comments, and the five
//!   standard entities — nothing else, on purpose.

pub mod export;

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use led_core::{DeviceId, PixelPhysical, RgbOrder};

// ── Minimal XML pull parser ───────────────────────────────────────────────────

/// One parsed XML element start tag: name + attributes (entities decoded).
#[derive(Debug, Clone)]
pub struct XmlElement {
    pub name: String,
    pub attrs: HashMap<String, String>,
    pub self_closing: bool,
}

/// Decode the five standard XML entities plus numeric character references.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let end = match rest.find(';') {
            Some(e) if e <= 12 => e,
            _ => {
                out.push('&');
                rest = &rest[1..];
                continue;
            }
        };
        let ent = &rest[1..end];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                if let Ok(cp) = u32::from_str_radix(&ent[2..], 16) {
                    if let Some(c) = char::from_u32(cp) {
                        out.push(c);
                    }
                }
            }
            _ if ent.starts_with('#') => {
                if let Ok(cp) = ent[1..].parse::<u32>() {
                    if let Some(c) = char::from_u32(cp) {
                        out.push(c);
                    }
                }
            }
            _ => {
                // unknown entity: keep verbatim
                out.push('&');
                out.push_str(ent);
                out.push(';');
            }
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Iterate the start tags (and self-closing tags) of an XML document.
/// Comments, declarations, end tags, and text content are skipped — the xLights
/// formats carry everything in attributes.
pub fn elements(xml: &str) -> Vec<XmlElement> {
    let mut out = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // comment / declaration / processing instruction
        if xml[i..].starts_with("<!--") {
            i = xml[i..].find("-->").map(|p| i + p + 3).unwrap_or(bytes.len());
            continue;
        }
        if xml[i..].starts_with("<?") || xml[i..].starts_with("<!") {
            i = xml[i..].find('>').map(|p| i + p + 1).unwrap_or(bytes.len());
            continue;
        }
        if xml[i..].starts_with("</") {
            i = xml[i..].find('>').map(|p| i + p + 1).unwrap_or(bytes.len());
            continue;
        }
        // start tag
        let close = match xml[i..].find('>') {
            Some(p) => i + p,
            None => break,
        };
        let inner = &xml[i + 1..close];
        let self_closing = inner.ends_with('/');
        let inner = inner.trim_end_matches('/').trim();
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let name = inner[..name_end].to_string();
        let mut attrs = HashMap::new();
        let mut rest = &inner[name_end..];
        loop {
            rest = rest.trim_start();
            let eq = match rest.find('=') {
                Some(e) => e,
                None => break,
            };
            let key = rest[..eq].trim().to_string();
            rest = rest[eq + 1..].trim_start();
            let quote = match rest.chars().next() {
                Some(q @ ('"' | '\'')) => q,
                _ => break,
            };
            let vend = match rest[1..].find(quote) {
                Some(v) => v + 1,
                None => break,
            };
            attrs.insert(key, decode_entities(&rest[1..vend]));
            rest = &rest[vend + 1..];
        }
        out.push(XmlElement { name, attrs, self_closing });
        i = close + 1;
    }
    out
}

// ── Networks (controllers) ────────────────────────────────────────────────────

/// One universe owned by a controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XUniverse {
    pub number: u16,
    pub channels: u16,
}

/// One controller from `xlights_networks.xml`.
#[derive(Debug, Clone)]
pub struct XController {
    pub name: String,
    pub ip: String,
    pub protocol: String, // "ArtNet" | "E131" | "DDP" | ...
    pub vendor: String,
    pub universes: Vec<XUniverse>,
}

impl XController {
    /// Total channel capacity of this controller.
    pub fn channel_capacity(&self) -> u32 {
        self.universes.iter().map(|u| u.channels as u32).sum()
    }

    /// Map a 1-based absolute controller channel to `(universe_number, 0-based channel)`.
    /// Returns `None` if the channel is beyond the controller's capacity.
    pub fn resolve_channel(&self, abs_channel_1based: u32) -> Option<(u16, u16)> {
        let mut remaining = abs_channel_1based.checked_sub(1)?;
        for u in &self.universes {
            if remaining < u.channels as u32 {
                return Some((u.number, remaining as u16));
            }
            remaining -= u.channels as u32;
        }
        None
    }
}

/// Parse `xlights_networks.xml`.
pub fn parse_networks(xml: &str) -> Vec<XController> {
    let mut controllers: Vec<XController> = Vec::new();
    for el in elements(xml) {
        match el.name.as_str() {
            "Controller" => controllers.push(XController {
                name: el.attrs.get("Name").cloned().unwrap_or_default(),
                ip: el.attrs.get("IP").cloned().unwrap_or_default(),
                protocol: el.attrs.get("Protocol").cloned().unwrap_or_default(),
                vendor: el.attrs.get("Vendor").cloned().unwrap_or_default(),
                universes: Vec::new(),
            }),
            "network" => {
                if let Some(c) = controllers.last_mut() {
                    // xLights stores the universe number in the (misnamed) BaudRate
                    // attribute for Ethernet controllers.
                    let number = el
                        .attrs
                        .get("BaudRate")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let channels = el
                        .attrs
                        .get("MaxChannels")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(510);
                    c.universes.push(XUniverse { number, channels });
                }
            }
            _ => {}
        }
    }
    controllers
}

// ── Models (strips) ───────────────────────────────────────────────────────────

/// One model (strip/prop) from `xlights_rgbeffects.xml`.
#[derive(Debug, Clone)]
pub struct XModel {
    pub name: String,
    pub display_as: String,
    pub controller: String,
    /// Raw StartChannel string, e.g. `!robô led 1:61` or `1234`.
    pub start_channel_raw: String,
    pub pixel_count: u32,
    pub string_type: String,
    pub world_x: f32,
    pub world_y: f32,
    pub world_z: f32,
    /// Second endpoint of the line, relative to the world position (xLights X2/Y2/Z2).
    pub x2: f32,
    pub y2: f32,
    pub z2: f32,
}

impl XModel {
    /// RGB order implied by the xLights StringType.
    pub fn rgb_order(&self) -> RgbOrder {
        let s = self.string_type.to_ascii_uppercase();
        if s.contains("GRB") {
            RgbOrder::Grb
        } else if s.contains("BGR") {
            RgbOrder::Bgr
        } else {
            RgbOrder::Rgb
        }
    }

    /// Channels this model occupies (3 per pixel).
    pub fn channel_count(&self) -> u32 {
        self.pixel_count * 3
    }

    /// World-space position of every pixel: linear interpolation from the
    /// world position to the second endpoint (`X2/Y2/Z2` are relative).
    pub fn pixel_positions(&self) -> Vec<(f32, f32, f32)> {
        let n = self.pixel_count.max(1);
        let denom = (n - 1).max(1) as f32;
        (0..n)
            .map(|i| {
                let t = i as f32 / denom;
                (
                    self.world_x + self.x2 * t,
                    self.world_y + self.y2 * t,
                    self.world_z + self.z2 * t,
                )
            })
            .collect()
    }

    /// Resolve the raw StartChannel: `(controller_name, 1-based absolute channel)`.
    /// Handles `!controller:N` and plain `N` (empty controller name).
    pub fn resolve_start(&self) -> Option<(String, u32)> {
        let raw = self.start_channel_raw.trim();
        if let Some(rest) = raw.strip_prefix('!') {
            let (ctrl, ch) = rest.rsplit_once(':')?;
            Some((ctrl.trim().to_string(), ch.trim().parse().ok()?))
        } else {
            Some((String::new(), raw.parse().ok()?))
        }
    }
}

/// A model group (hierarchy) from `xlights_rgbeffects.xml`.
#[derive(Debug, Clone)]
pub struct XGroup {
    pub name: String,
    pub members: Vec<String>,
}

/// Parse `xlights_rgbeffects.xml` — models and groups.
pub fn parse_rgbeffects(xml: &str) -> (Vec<XModel>, Vec<XGroup>) {
    let mut models = Vec::new();
    let mut groups = Vec::new();
    for el in elements(xml) {
        match el.name.as_str() {
            "model" => {
                let get = |k: &str| el.attrs.get(k).cloned().unwrap_or_default();
                let num = |k: &str| -> u32 {
                    el.attrs
                        .get(k)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0)
                };
                let fnum = |k: &str| -> f32 {
                    el.attrs
                        .get(k)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0.0)
                };
                // Pixel count: strings × nodes × lights-per-node (Single Line etc.).
                // CustomModel grids are counted by their non-empty cells.
                let mut pixels = num("NumStrings").max(1) * num("NodesPerString")
                    * num("LightsPerNode").max(1);
                if pixels == 0 {
                    if let Some(cm) = el.attrs.get("CustomModel") {
                        pixels = cm
                            .split(';')
                            .flat_map(|row| row.split(','))
                            .filter(|c| !c.trim().is_empty())
                            .count() as u32;
                    }
                }
                models.push(XModel {
                    name: get("name"),
                    display_as: get("DisplayAs"),
                    controller: get("Controller"),
                    start_channel_raw: get("StartChannel"),
                    pixel_count: pixels,
                    string_type: get("StringType"),
                    world_x: fnum("WorldPosX"),
                    world_y: fnum("WorldPosY"),
                    world_z: fnum("WorldPosZ"),
                    x2: fnum("X2"),
                    y2: fnum("Y2"),
                    z2: fnum("Z2"),
                });
            }
            "modelGroup" => {
                groups.push(XGroup {
                    name: el.attrs.get("name").cloned().unwrap_or_default(),
                    members: el
                        .attrs
                        .get("models")
                        .map(|m| {
                            m.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default(),
                });
            }
            _ => {}
        }
    }
    (models, groups)
}

// ── Conflict gate ─────────────────────────────────────────────────────────────

/// Two models whose channel ranges overlap on the same controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConflict {
    pub controller: String,
    pub model_a: String,
    pub model_b: String,
    /// 1-based absolute channel range of the overlap (inclusive start, exclusive end).
    pub overlap: (u32, u32),
}

impl fmt::Display for ChannelConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: '{}' overlaps '{}' on channels {}..{}",
            self.controller, self.model_a, self.model_b, self.overlap.0, self.overlap.1
        )
    }
}

/// Errors surfaced by the import gate.
#[derive(Debug)]
pub enum ImportError {
    /// Channel conflicts exist — layout is ambiguous, refuse to compile.
    Conflicts(Vec<ChannelConflict>),
    /// A model references a controller absent from networks.xml.
    UnknownController { model: String, controller: String },
    /// A model's channels exceed its controller's universe capacity.
    BeyondCapacity { model: String, controller: String, channel: u32 },
    /// A model's StartChannel could not be parsed.
    BadStartChannel { model: String, raw: String },
    Io(std::io::Error),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflicts(c) => write!(f, "{} channel conflicts — fix the layout first", c.len()),
            Self::UnknownController { model, controller } => {
                write!(f, "model '{model}' references unknown controller '{controller}'")
            }
            Self::BeyondCapacity { model, controller, channel } => write!(
                f,
                "model '{model}' channel {channel} exceeds capacity of '{controller}'"
            ),
            Self::BadStartChannel { model, raw } => {
                write!(f, "model '{model}' has unparseable StartChannel '{raw}'")
            }
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

// ── ImportReport ──────────────────────────────────────────────────────────────

/// The result of importing an xLights show folder.
#[derive(Debug)]
pub struct ImportReport {
    pub controllers: Vec<XController>,
    pub models: Vec<XModel>,
    pub groups: Vec<XGroup>,
    pub conflicts: Vec<ChannelConflict>,
}

impl ImportReport {
    /// Total pixels across all models.
    pub fn total_pixels(&self) -> u32 {
        self.models.iter().map(|m| m.pixel_count).sum()
    }

    /// The gate: physical assignments for `CompiledLayout::compile`, one entry
    /// per pixel, ordered model-by-model. `Err(Conflicts)` while overlaps exist.
    ///
    /// Device ids are assigned by controller order in networks.xml (0-based).
    pub fn assignments(&self) -> Result<Vec<PixelPhysical>, ImportError> {
        if !self.conflicts.is_empty() {
            return Err(ImportError::Conflicts(self.conflicts.clone()));
        }
        let mut out = Vec::with_capacity(self.total_pixels() as usize);
        for m in &self.models {
            let (ctrl_name, start) = m.resolve_start().ok_or_else(|| {
                ImportError::BadStartChannel { model: m.name.clone(), raw: m.start_channel_raw.clone() }
            })?;
            let (dev_id, ctrl) = self
                .controllers
                .iter()
                .enumerate()
                .find(|(_, c)| c.name == ctrl_name)
                .map(|(i, c)| (i as DeviceId, c))
                .ok_or_else(|| ImportError::UnknownController {
                    model: m.name.clone(),
                    controller: ctrl_name.clone(),
                })?;
            let order = m.rgb_order();
            for px in 0..m.pixel_count {
                let abs = start + px * 3;
                let (universe, channel) =
                    ctrl.resolve_channel(abs).ok_or_else(|| ImportError::BeyondCapacity {
                        model: m.name.clone(),
                        controller: ctrl_name.clone(),
                        channel: abs,
                    })?;
                out.push(PixelPhysical { device: dev_id, universe, channel, format: order.into() });
            }
        }
        Ok(out)
    }

    /// One-line JSON summary for logs.
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"controllers":{},"models":{},"groups":{},"pixels":{},"conflicts":{}}}"#,
            self.controllers.len(),
            self.models.len(),
            self.groups.len(),
            self.total_pixels(),
            self.conflicts.len(),
        )
    }
}

/// Detect channel-range overlaps between models on the same controller.
pub fn find_channel_conflicts(
    models: &[XModel],
    controllers: &[XController],
) -> Vec<ChannelConflict> {
    // (controller, start, end, model) — 1-based absolute channels, end exclusive
    let mut ranges: Vec<(String, u32, u32, String)> = Vec::new();
    for m in models {
        if let Some((ctrl, start)) = m.resolve_start() {
            // Only gate models on known controllers; unknown ones error later.
            if controllers.iter().any(|c| c.name == ctrl) || ctrl.is_empty() {
                ranges.push((ctrl, start, start + m.channel_count(), m.name.clone()));
            }
        }
    }
    ranges.sort();
    let mut conflicts = Vec::new();
    for i in 0..ranges.len() {
        for j in i + 1..ranges.len() {
            let (ca, sa, ea, na) = &ranges[i];
            let (cb, sb, eb, nb) = &ranges[j];
            if ca != cb {
                break; // sorted by controller: no more pairs for i
            }
            if sb >= ea {
                break; // sorted by start: no later range can overlap i
            }
            conflicts.push(ChannelConflict {
                controller: ca.clone(),
                model_a: na.clone(),
                model_b: nb.clone(),
                overlap: (*sb.max(sa), *ea.min(eb)),
            });
        }
    }
    conflicts
}

// ── Groups → pixel ranges ─────────────────────────────────────────────────────

impl ImportReport {
    /// Pixel index ranges (into the `assignments()` order) covered by a model
    /// group. Groups may nest (the user's rig has `robôs T` → `robô rN` →
    /// models); membership is resolved recursively, cycles ignored.
    pub fn pixels_for_group(&self, group: &str) -> Vec<std::ops::Range<usize>> {
        // Pixel offset of each model, in assignments() order.
        let mut offsets = HashMap::new();
        let mut cursor = 0usize;
        for m in &self.models {
            offsets.insert(m.name.as_str(), cursor..cursor + m.pixel_count as usize);
            cursor += m.pixel_count as usize;
        }
        let by_name: HashMap<&str, &XGroup> =
            self.groups.iter().map(|g| (g.name.as_str(), g)).collect();

        let mut out = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![group];
        while let Some(name) = stack.pop() {
            if !visited.insert(name.to_string()) {
                continue; // cycle guard
            }
            if let Some(r) = offsets.get(name) {
                out.push(r.clone());
            } else if let Some(g) = by_name.get(name) {
                for m in &g.members {
                    stack.push(m.as_str());
                }
            }
        }
        out.sort_by_key(|r| r.start);
        out
    }
}

// ── Sequence (.xsq) timing import ─────────────────────────────────────────────

/// One effect placement from an xLights sequence: which element, which effect,
/// and when. Settings strings (EffectDB) are not imported in this slice —
/// timing and targeting are the structure a LUMYX `Timeline` needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectSpan {
    pub element: String,
    pub effect: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// A parsed `.xsq` sequence (timing slice).
#[derive(Debug, Clone)]
pub struct XSequence {
    pub media: Option<String>,
    pub duration_ms: u64,
    pub spans: Vec<EffectSpan>,
}

/// First text content of `<tag>…</tag>` (xLights head fields are text nodes).
fn text_of(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(decode_entities(xml[start..end].trim()))
}

/// Parse an xLights `.xsq` sequence file (timing slice: element → effect spans).
pub fn parse_sequence(xml: &str) -> XSequence {
    let media = text_of(xml, "mediaFile").filter(|s| !s.is_empty());
    let duration_ms = text_of(xml, "sequenceDuration")
        .and_then(|s| s.parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as u64)
        .unwrap_or(0);

    // Effects appear nested inside their Element; the flat scan tracks the
    // last-seen model Element (xLights files are machine-generated and
    // well-formed, so "last seen" is the parent).
    let mut spans = Vec::new();
    let mut current: Option<String> = None;
    for el in elements(xml) {
        match el.name.as_str() {
            "Element" => {
                current = (el.attrs.get("type").map(String::as_str) == Some("model"))
                    .then(|| el.attrs.get("name").cloned().unwrap_or_default());
            }
            "Effect" => {
                if let Some(element) = &current {
                    let num = |k: &str| -> Option<u64> {
                        el.attrs.get(k).and_then(|v| v.parse().ok())
                    };
                    if let (Some(start_ms), Some(end_ms)) = (num("startTime"), num("endTime")) {
                        spans.push(EffectSpan {
                            element: element.clone(),
                            effect: el
                                .attrs
                                .get("name")
                                .cloned()
                                .unwrap_or_else(|| "unknown".into()),
                            start_ms,
                            end_ms,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    XSequence { media, duration_ms, spans }
}

/// Load and parse a `.xsq` from disk.
pub fn parse_sequence_file(path: &Path) -> Result<XSequence, std::io::Error> {
    Ok(parse_sequence(&std::fs::read_to_string(path)?))
}

// ── Auto-fix: conflict-free channel repacking ─────────────────────────────────

/// A proposed new start channel for one model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelFix {
    pub model: String,
    pub controller: String,
    pub old_start: u32,
    pub new_start: u32,
}

impl ImportReport {
    /// Propose a conflict-free repacking: per controller, models keep their
    /// current relative order (by start channel, then name — stable) and are
    /// repacked contiguously from channel 1. The result is overlap-free by
    /// construction and always fits if the controller had capacity.
    ///
    /// This replaces the by-hand channel arithmetic that xLights pushes onto
    /// users (the "12 Python scripts" workflow).
    pub fn propose_fix(&self) -> Vec<ChannelFix> {
        // controller → [(old_start, name, channels)]
        let mut per_ctrl: HashMap<String, Vec<(u32, String, u32)>> = HashMap::new();
        for m in &self.models {
            if let Some((ctrl, start)) = m.resolve_start() {
                per_ctrl
                    .entry(ctrl)
                    .or_default()
                    .push((start, m.name.clone(), m.channel_count()));
            }
        }
        let mut fixes = Vec::new();
        let mut ctrls: Vec<_> = per_ctrl.into_iter().collect();
        ctrls.sort_by(|a, b| a.0.cmp(&b.0));
        for (ctrl, mut models) in ctrls {
            models.sort(); // by old_start, then name — stable, preserves intent
            let mut next: u32 = 1;
            for (old_start, name, channels) in models {
                if old_start != next {
                    fixes.push(ChannelFix {
                        model: name,
                        controller: ctrl.clone(),
                        old_start,
                        new_start: next,
                    });
                }
                next += channels;
            }
        }
        fixes
    }
}

/// Rewrite `StartChannel` attributes in a raw `xlights_rgbeffects.xml` string,
/// applying `fixes`. Everything else in the file — effects, groups, positions,
/// comments — is preserved byte-for-byte. Returns the new XML.
///
/// Write the result to a **new** file; never overwrite the user's original.
pub fn apply_fixes_to_xml(xml: &str, fixes: &[ChannelFix]) -> String {
    let by_model: HashMap<&str, &ChannelFix> =
        fixes.iter().map(|f| (f.model.as_str(), f)).collect();
    let mut out = String::with_capacity(xml.len());
    let mut i = 0;
    while i < xml.len() {
        let Some(tag_rel) = xml[i..].find("<model ") else {
            out.push_str(&xml[i..]);
            break;
        };
        let tag_start = i + tag_rel;
        let tag_end = match xml[tag_start..].find('>') {
            Some(p) => tag_start + p + 1,
            None => {
                out.push_str(&xml[i..]);
                break;
            }
        };
        out.push_str(&xml[i..tag_start]);
        let tag = &xml[tag_start..tag_end];

        // Which model is this? (name attribute, entities decoded.)
        let name = elements(tag)
            .first()
            .and_then(|el| el.attrs.get("name").cloned())
            .unwrap_or_default();

        if let Some(fix) = by_model.get(name.as_str()) {
            // Replace the StartChannel="..." value inside this tag only.
            if let Some(sc_rel) = tag.find("StartChannel=\"") {
                let val_start = sc_rel + "StartChannel=\"".len();
                if let Some(val_len) = tag[val_start..].find('"') {
                    let new_val = format!("!{}:{}", fix.controller, fix.new_start);
                    out.push_str(&tag[..val_start]);
                    out.push_str(&new_val);
                    out.push_str(&tag[val_start + val_len..]);
                    i = tag_end;
                    continue;
                }
            }
        }
        out.push_str(tag);
        i = tag_end;
    }
    out
}

/// Import an xLights show folder (`xlights_networks.xml` + `xlights_rgbeffects.xml`).
pub fn import_show_dir(dir: &Path) -> Result<ImportReport, ImportError> {
    let networks =
        std::fs::read_to_string(dir.join("xlights_networks.xml")).map_err(ImportError::Io)?;
    let rgbeffects =
        std::fs::read_to_string(dir.join("xlights_rgbeffects.xml")).map_err(ImportError::Io)?;
    Ok(import_strings(&networks, &rgbeffects))
}

/// Import from already-loaded XML strings (testable without a filesystem).
pub fn import_strings(networks_xml: &str, rgbeffects_xml: &str) -> ImportReport {
    let controllers = parse_networks(networks_xml);
    let (models, groups) = parse_rgbeffects(rgbeffects_xml);
    let conflicts = find_channel_conflicts(&models, &controllers);
    ImportReport { controllers, models, groups, conflicts }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const NETWORKS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Networks computer="test">
  <Controller Name="rob&#244; led 1" Type="Ethernet" Vendor="WLED" IP="192.168.2.156" Protocol="ArtNet">
    <network ComPort="192.168.2.156" BaudRate="1" NetworkType="ArtNet" MaxChannels="510" />
    <network ComPort="192.168.2.156" BaudRate="2" NetworkType="ArtNet" MaxChannels="510" />
  </Controller>
  <Controller Name="ctrl B" Type="Ethernet" Vendor="Falcon" IP="10.0.0.2" Protocol="E131">
    <network ComPort="10.0.0.2" BaudRate="7" NetworkType="E131" MaxChannels="512" />
  </Controller>
</Networks>"#;

    fn model_xml(name: &str, ctrl: &str, start: u32, nodes: u32) -> String {
        format!(
            r#"<model name="{name}" DisplayAs="Single Line" Controller="{ctrl}" NumStrings="1" NodesPerString="{nodes}" LightsPerNode="1" StringType="RGB Nodes" StartChannel="!{ctrl}:{start}" WorldPosX="1.5" WorldPosY="2.5" WorldPosZ="0" />"#
        )
    }

    fn wrap_effects(models: &str, groups: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><xrgb><models>{models}</models><modelGroups>{groups}</modelGroups></xrgb>"#
        )
    }

    // ── XML parser ────────────────────────────────────────────────────────────

    #[test]
    fn parser_handles_entities_and_self_closing() {
        let els = elements(r#"<a name="R&amp;B &quot;x&quot; rob&#244;"/><b k='v'/>"#);
        assert_eq!(els.len(), 2);
        assert_eq!(els[0].attrs["name"], "R&B \"x\" robô");
        assert!(els[0].self_closing);
        assert_eq!(els[1].attrs["k"], "v");
    }

    #[test]
    fn parser_skips_comments_and_declarations() {
        let els = elements("<?xml version=\"1.0\"?><!-- <fake attr=\"1\"/> --><real a=\"2\"/>");
        assert_eq!(els.len(), 1);
        assert_eq!(els[0].name, "real");
    }

    // ── Networks ──────────────────────────────────────────────────────────────

    #[test]
    fn networks_parse_controllers_and_universes() {
        let ctrls = parse_networks(NETWORKS);
        assert_eq!(ctrls.len(), 2);
        assert_eq!(ctrls[0].name, "robô led 1");
        assert_eq!(ctrls[0].ip, "192.168.2.156");
        assert_eq!(ctrls[0].protocol, "ArtNet");
        assert_eq!(
            ctrls[0].universes,
            vec![XUniverse { number: 1, channels: 510 }, XUniverse { number: 2, channels: 510 }]
        );
        assert_eq!(ctrls[1].universes, vec![XUniverse { number: 7, channels: 512 }]);
    }

    #[test]
    fn controller_resolves_absolute_channel_across_universes() {
        let ctrls = parse_networks(NETWORKS);
        let c = &ctrls[0];
        assert_eq!(c.resolve_channel(1), Some((1, 0)), "first channel");
        assert_eq!(c.resolve_channel(510), Some((1, 509)), "last of universe 1");
        assert_eq!(c.resolve_channel(511), Some((2, 0)), "first of universe 2");
        assert_eq!(c.resolve_channel(1020), Some((2, 509)), "last channel");
        assert_eq!(c.resolve_channel(1021), None, "beyond capacity");
        assert_eq!(c.resolve_channel(0), None, "channels are 1-based");
    }

    // ── Models ────────────────────────────────────────────────────────────────

    #[test]
    fn rgbeffects_parse_models_and_groups() {
        let xml = wrap_effects(
            &(model_xml("strip A", "c1", 1, 20) + &model_xml("strip B", "c1", 61, 10)),
            r#"<modelGroup name="all" models="strip A,strip B"/>"#,
        );
        let (models, groups) = parse_rgbeffects(&xml);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].pixel_count, 20);
        assert_eq!(models[0].world_x, 1.5);
        assert_eq!(models[1].resolve_start(), Some(("c1".into(), 61)));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, vec!["strip A", "strip B"]);
    }

    #[test]
    fn pixel_positions_interpolate_along_the_line() {
        let mut m = XModel {
            name: "line".into(),
            display_as: "Single Line".into(),
            controller: String::new(),
            start_channel_raw: "1".into(),
            pixel_count: 3,
            string_type: "RGB Nodes".into(),
            world_x: 10.0,
            world_y: 20.0,
            world_z: 0.0,
            x2: 4.0,
            y2: 0.0,
            z2: 0.0,
        };
        let pos = m.pixel_positions();
        assert_eq!(pos.len(), 3);
        assert_eq!(pos[0], (10.0, 20.0, 0.0), "first pixel at world pos");
        assert_eq!(pos[1], (12.0, 20.0, 0.0), "middle of the segment");
        assert_eq!(pos[2], (14.0, 20.0, 0.0), "second endpoint = world + X2");

        m.pixel_count = 1; // degenerate single-pixel strip must not divide by zero
        assert_eq!(m.pixel_positions(), vec![(10.0, 20.0, 0.0)]);
    }

    #[test]
    fn string_type_maps_to_rgb_order() {
        let mk = |st: &str| XModel {
            name: "m".into(),
            display_as: "Single Line".into(),
            controller: String::new(),
            start_channel_raw: "1".into(),
            pixel_count: 1,
            string_type: st.into(),
            world_x: 0.0,
            world_y: 0.0,
            world_z: 0.0,
            x2: 0.0,
            y2: 0.0,
            z2: 0.0,
        };
        assert_eq!(mk("RGB Nodes").rgb_order(), RgbOrder::Rgb);
        assert_eq!(mk("GRB Nodes").rgb_order(), RgbOrder::Grb);
        assert_eq!(mk("BGR Nodes").rgb_order(), RgbOrder::Bgr);
    }

    // ── Conflict gate ─────────────────────────────────────────────────────────

    #[test]
    fn overlapping_models_are_detected() {
        // strip A: channels 1..61 (20px). strip B: 30..90 — overlaps 30..61.
        let xml = wrap_effects(
            &(model_xml("strip A", "robô led 1", 1, 20)
                + &model_xml("strip B", "robô led 1", 30, 20)),
            "",
        );
        let report = import_strings(NETWORKS, &xml);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].overlap, (30, 61));
        assert!(matches!(report.assignments(), Err(ImportError::Conflicts(_))),
            "gate must refuse while conflicts exist");
    }

    #[test]
    fn adjacent_models_do_not_conflict() {
        // A: 1..61, B: 61..121 — touching, not overlapping.
        let xml = wrap_effects(
            &(model_xml("A", "robô led 1", 1, 20) + &model_xml("B", "robô led 1", 61, 20)),
            "",
        );
        let report = import_strings(NETWORKS, &xml);
        assert!(report.conflicts.is_empty(), "adjacent ranges are legal");
    }

    #[test]
    fn same_channels_on_different_controllers_do_not_conflict() {
        let xml = wrap_effects(
            &(model_xml("A", "robô led 1", 1, 20) + &model_xml("B", "ctrl B", 1, 20)),
            "",
        );
        let report = import_strings(NETWORKS, &xml);
        assert!(report.conflicts.is_empty());
    }

    // ── Assignments ───────────────────────────────────────────────────────────

    #[test]
    fn assignments_produce_correct_physical_slots() {
        // 20px starting at channel 505 of universe 1 (510ch): pixels straddle
        // the universe boundary — px0 at (u1,504), px2 begins in universe 2.
        let xml = wrap_effects(&model_xml("edge", "robô led 1", 505, 20), "");
        let report = import_strings(NETWORKS, &xml);
        let assigns = report.assignments().expect("no conflicts");
        assert_eq!(assigns.len(), 20);
        assert_eq!((assigns[0].universe, assigns[0].channel), (1, 504));
        // px1 abs = 505+3 = 508 → (1, 507); px2 abs = 511 → (2, 0)
        assert_eq!((assigns[1].universe, assigns[1].channel), (1, 507));
        assert_eq!((assigns[2].universe, assigns[2].channel), (2, 0));
        assert_eq!(assigns[0].device, 0, "device id = controller order");
    }

    #[test]
    fn assignments_feed_compiled_layout() {
        let xml = wrap_effects(
            &(model_xml("A", "robô led 1", 1, 10) + &model_xml("B", "ctrl B", 1, 5)),
            "",
        );
        let report = import_strings(NETWORKS, &xml);
        let assigns = report.assignments().unwrap();
        let layout = led_core::CompiledLayout::compile(&assigns);
        // 2 devices, 15 pixels total — layout must be constructible and coherent
        assert_eq!(assigns.len(), 15);
        assert!(!layout.device_universes(0).is_empty());
        assert!(!layout.device_universes(1).is_empty());
    }

    #[test]
    fn unknown_controller_is_an_error_not_a_silent_skip() {
        let xml = wrap_effects(&model_xml("ghost", "no such ctrl", 1, 5), "");
        let report = import_strings(NETWORKS, &xml);
        assert!(matches!(
            report.assignments(),
            Err(ImportError::UnknownController { .. })
        ));
    }

    #[test]
    fn beyond_capacity_is_an_error() {
        // ctrl B has 1×512 channels; 200px needs 600.
        let xml = wrap_effects(&model_xml("big", "ctrl B", 1, 200), "");
        let report = import_strings(NETWORKS, &xml);
        assert!(matches!(
            report.assignments(),
            Err(ImportError::BeyondCapacity { .. })
        ));
    }

    // ── Groups → pixels ───────────────────────────────────────────────────────

    #[test]
    fn pixels_for_group_resolves_nested_groups() {
        let xml = wrap_effects(
            &(model_xml("a", "robô led 1", 1, 10)
                + &model_xml("b", "robô led 1", 31, 10)
                + &model_xml("c", "robô led 1", 61, 10)),
            r#"<modelGroup name="inner" models="a,b"/>
               <modelGroup name="outer" models="inner,c"/>
               <modelGroup name="loop" models="loop,a"/>"#,
        );
        let report = import_strings(NETWORKS, &xml);
        // a → 0..10, b → 10..20, c → 20..30 (assignments order)
        assert_eq!(report.pixels_for_group("inner"), vec![0..10, 10..20]);
        assert_eq!(report.pixels_for_group("outer"), vec![0..10, 10..20, 20..30],
            "nested group resolves recursively");
        assert_eq!(report.pixels_for_group("loop"), vec![0..10], "cycles are ignored");
        assert!(report.pixels_for_group("missing").is_empty());
    }

    // ── Sequence (.xsq) ───────────────────────────────────────────────────────

    #[test]
    fn sequence_timing_parses_element_effect_spans() {
        let xsq = r#"<?xml version="1.0"?><xsequence>
          <head><mediaFile>/music/beat.mp3</mediaFile><sequenceDuration>157.387</sequenceDuration></head>
          <EffectDB><Effect>E_SETTINGS=1</Effect></EffectDB>
          <ElementEffects>
            <Element type="timing" name="New Timing">
              <EffectLayer><Effect name="timing-mark" startTime="0" endTime="100"/></EffectLayer>
            </Element>
            <Element type="model" name="rob&#244;s T">
              <EffectLayer>
                <Effect ref="0" name="Meteors" startTime="86300" endTime="93300" palette="0"/>
                <Effect ref="1" name="Lightning" startTime="93300" endTime="99000"/>
              </EffectLayer>
            </Element>
          </ElementEffects>
        </xsequence>"#;
        let seq = parse_sequence(xsq);
        assert_eq!(seq.media.as_deref(), Some("/music/beat.mp3"));
        assert_eq!(seq.duration_ms, 157_387);
        assert_eq!(seq.spans.len(), 2, "timing-track effects are not model spans");
        assert_eq!(seq.spans[0], EffectSpan {
            element: "robôs T".into(), effect: "Meteors".into(),
            start_ms: 86_300, end_ms: 93_300,
        });
        assert_eq!(seq.spans[1].effect, "Lightning");
    }

    #[test]
    fn empty_sequence_yields_no_spans() {
        let seq = parse_sequence(
            r#"<xsequence><head><sequenceDuration>10.0</sequenceDuration></head></xsequence>"#,
        );
        assert_eq!(seq.duration_ms, 10_000);
        assert!(seq.spans.is_empty());
        assert!(seq.media.is_none());
    }

    // ── Auto-fix ──────────────────────────────────────────────────────────────

    #[test]
    fn propose_fix_removes_all_conflicts() {
        // Three models all starting at channel 1 — the user's real pattern.
        let xml = wrap_effects(
            &(model_xml("A", "robô led 1", 1, 20)
                + &model_xml("B", "robô led 1", 1, 20)
                + &model_xml("C", "robô led 1", 1, 20)),
            "",
        );
        let report = import_strings(NETWORKS, &xml);
        assert!(!report.conflicts.is_empty(), "fixture must start broken");

        let fixes = report.propose_fix();
        // Apply fixes to the models and re-check: zero conflicts.
        let mut fixed_models = report.models.clone();
        for f in &fixes {
            let m = fixed_models.iter_mut().find(|m| m.name == f.model).unwrap();
            m.start_channel_raw = format!("!{}:{}", f.controller, f.new_start);
        }
        let after = find_channel_conflicts(&fixed_models, &report.controllers);
        assert!(after.is_empty(), "repacked layout must be conflict-free: {after:?}");
        // Repacking is contiguous: A=1, B=61, C=121 (20px × 3ch).
        let starts: Vec<u32> =
            fixed_models.iter().map(|m| m.resolve_start().unwrap().1).collect();
        assert_eq!(starts, vec![1, 61, 121]);
    }

    #[test]
    fn propose_fix_preserves_relative_order() {
        let xml = wrap_effects(
            &(model_xml("early", "robô led 1", 100, 10)
                + &model_xml("late", "robô led 1", 100, 10)), // tie → name order
            "",
        );
        let report = import_strings(NETWORKS, &xml);
        let fixes = report.propose_fix();
        let early = fixes.iter().find(|f| f.model == "early").unwrap();
        let late = fixes.iter().find(|f| f.model == "late").unwrap();
        assert!(early.new_start < late.new_start, "stable order by (start, name)");
    }

    #[test]
    fn apply_fixes_rewrites_only_start_channels() {
        let xml = wrap_effects(
            &(model_xml("A", "robô led 1", 1, 20) + &model_xml("B", "robô led 1", 1, 20)),
            r#"<modelGroup name="all" models="A,B"/>"#,
        );
        let report = import_strings(NETWORKS, &xml);
        let fixes = report.propose_fix();
        let fixed_xml = apply_fixes_to_xml(&xml, &fixes);

        // Re-import the fixed XML: conflicts must be gone, everything else intact.
        let report2 = import_strings(NETWORKS, &fixed_xml);
        assert!(report2.conflicts.is_empty(), "rewritten XML must pass the gate");
        assert_eq!(report2.models.len(), 2);
        assert_eq!(report2.groups.len(), 1, "groups untouched");
        assert_eq!(report2.total_pixels(), 40, "pixel counts untouched");
        assert!(report2.assignments().is_ok());
    }

    #[test]
    fn apply_fixes_preserves_unrelated_models_byte_for_byte() {
        let xml = wrap_effects(
            &(model_xml("keep", "ctrl B", 1, 5) + &model_xml("fix me", "robô led 1", 1, 20)
                + &model_xml("fix me 2", "robô led 1", 1, 20)),
            "",
        );
        let report = import_strings(NETWORKS, &xml);
        let fixed = apply_fixes_to_xml(&xml, &report.propose_fix());
        // 'keep' has no conflicts and no fix — its tag must be byte-identical.
        let orig_tag = xml.split("<model ").find(|t| t.contains("\"keep\"")).unwrap();
        assert!(fixed.contains(orig_tag), "untouched model preserved verbatim");
    }

    #[test]
    fn report_json_summary() {
        let xml = wrap_effects(&model_xml("A", "robô led 1", 1, 20), "");
        let report = import_strings(NETWORKS, &xml);
        let j = report.to_json();
        assert!(j.contains("\"controllers\":2"));
        assert!(j.contains("\"pixels\":20"));
        assert!(j.contains("\"conflicts\":0"));
    }
}
