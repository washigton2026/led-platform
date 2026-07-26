//! xLights export — the other direction of the bridge.
//!
//! `led-xlights` imports xLights projects; this module writes them, so a rig
//! built with `led_layout::build_rig` (conflict-free by construction) can be
//! opened in xLights. The loop closes: LUMYX → xLights file → re-import →
//! the same channel-conflict gate that guards imports guards our own output.
//!
//! ## Invariants
//! - Exported XML re-imports through [`crate::parse_rgbeffects`] with every
//!   field intact (round-trip is a test, not a hope).
//! - Attribute values are entity-escaped; a model named `R&B "x"` must survive.
//! - Exported layouts pass the conflict gate — we never emit a file we would
//!   refuse to import.

use std::fmt::Write as _;

use led_core::DeviceId;
use led_layout::RigPlan;

use crate::{XGroup, XModel};

/// Escape the five XML-special characters for attribute values.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Serialize models + groups as an `xlights_rgbeffects.xml` document that
/// xLights (and [`crate::parse_rgbeffects`]) can read.
pub fn export_rgbeffects(models: &[XModel], groups: &[XGroup]) -> String {
    let mut xml = String::with_capacity(models.len() * 256 + 512);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<xrgb SourceVersion=\"LUMYX\">\n");
    xml.push_str("  <models>\n");
    for m in models {
        let _ = writeln!(
            xml,
            "    <model name=\"{name}\" DisplayAs=\"{display}\" Controller=\"{ctrl}\" \
             StringType=\"{st}\" NumStrings=\"1\" NodesPerString=\"{px}\" LightsPerNode=\"1\" \
             StartChannel=\"{sc}\" WorldPosX=\"{wx}\" WorldPosY=\"{wy}\" WorldPosZ=\"{wz}\" \
             X2=\"{x2}\" Y2=\"{y2}\" Z2=\"{z2}\" />",
            name = escape(&m.name),
            display = escape(&m.display_as),
            ctrl = escape(&m.controller),
            st = escape(&m.string_type),
            px = m.pixel_count,
            sc = escape(&m.start_channel_raw),
            wx = m.world_x,
            wy = m.world_y,
            wz = m.world_z,
            x2 = m.x2,
            y2 = m.y2,
            z2 = m.z2,
        );
    }
    xml.push_str("  </models>\n  <modelGroups>\n");
    for g in groups {
        let _ = writeln!(
            xml,
            "    <modelGroup name=\"{}\" models=\"{}\" />",
            escape(&g.name),
            escape(&g.members.join(",")),
        );
    }
    xml.push_str("  </modelGroups>\n</xrgb>\n");
    xml
}

/// Convert a [`RigPlan`] into exportable [`XModel`]s + per-instance groups.
///
/// `controller_names[d]` names the controller for device id `first_device + d`
/// (one controller per rig instance, mirroring the real 5-robot rig). Strip
/// geometry: a horizontal line of `pixels` units starting at the placed
/// world position (xLights re-scales freely; channels are what must be exact).
pub fn rig_to_xmodels(
    plan: &RigPlan,
    controller_names: &[&str],
) -> (Vec<XModel>, Vec<XGroup>) {
    // Absolute 1-based controller channel: (universe - first_universe_of_device)
    // × 510 + channel + 1. First universe per device = min seen in the plan.
    let mut first_uni: std::collections::HashMap<DeviceId, u16> = std::collections::HashMap::new();
    for s in &plan.strips {
        first_uni
            .entry(s.device)
            .and_modify(|u| *u = (*u).min(s.universe))
            .or_insert(s.universe);
    }
    let min_device = plan.strips.iter().map(|s| s.device).min().unwrap_or(0);

    let mut models = Vec::with_capacity(plan.strips.len());
    let mut groups: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for s in &plan.strips {
        let ctrl_idx = (s.device - min_device) as usize;
        let ctrl = controller_names.get(ctrl_idx).copied().unwrap_or("lumyx");
        let abs =
            (s.universe - first_uni[&s.device]) as u32 * 510 + s.channel as u32 + 1;
        models.push(XModel {
            name: s.name.clone(),
            display_as: "Single Line".into(),
            controller: ctrl.to_string(),
            start_channel_raw: format!("!{ctrl}:{abs}"),
            pixel_count: s.pixels as u32,
            string_type: match plan.order {
                led_core::RgbOrder::Grb => "GRB Nodes".into(),
                led_core::RgbOrder::Bgr => "BGR Nodes".into(),
                led_core::RgbOrder::Rgb => "RGB Nodes".into(),
            },
            world_x: s.world.0,
            world_y: s.world.1,
            world_z: s.world.2,
            x2: s.pixels as f32, // unit-per-pixel horizontal line
            y2: 0.0,
            z2: 0.0,
        });
        // One group per instance ("<template> <n>"): strip names are
        // "<template> <n>/<strip>" — group by the prefix before '/'.
        if let Some((instance, _)) = s.name.split_once('/') {
            groups
                .entry(instance.to_string())
                .or_default()
                .push(s.name.clone());
        }
    }
    let groups = groups
        .into_iter()
        .map(|(name, members)| XGroup { name, members })
        .collect();
    (models, groups)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{find_channel_conflicts, import_strings, parse_rgbeffects};
    use led_core::RgbOrder;
    use led_layout::{build_rig, RigTemplate, StripTemplate};

    fn networks_for(names: &[&str], universes: u16) -> String {
        let mut xml = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Networks computer=\"t\">\n",
        );
        for n in names {
            xml.push_str(&format!(
                "<Controller Name=\"{n}\" Type=\"Ethernet\" Vendor=\"WLED\" \
                 IP=\"10.0.0.1\" Protocol=\"ArtNet\">\n"
            ));
            for u in 1..=universes {
                xml.push_str(&format!(
                    "<network ComPort=\"10.0.0.1\" BaudRate=\"{u}\" \
                     NetworkType=\"ArtNet\" MaxChannels=\"510\" />\n"
                ));
            }
            xml.push_str("</Controller>\n");
        }
        xml.push_str("</Networks>\n");
        xml
    }

    fn mini_rig() -> RigPlan {
        let template = RigTemplate {
            name: "robô".into(),
            strips: vec![
                StripTemplate { name: "peito".into(), pixels: 20, offset: (0.0, 10.0, 0.0) },
                StripTemplate { name: "perna".into(), pixels: 30, offset: (1.0, 0.0, 0.0) },
            ],
        };
        build_rig(&template, 2, 0, 1, RgbOrder::Grb, (600.0, 0.0, 0.0))
    }

    #[test]
    fn export_reimport_roundtrip_preserves_every_field() {
        let (models, groups) = rig_to_xmodels(&mini_rig(), &["robô led 1", "robô led 2"]);
        let xml = export_rgbeffects(&models, &groups);
        let (back_models, back_groups) = parse_rgbeffects(&xml);

        assert_eq!(back_models.len(), models.len());
        for (a, b) in models.iter().zip(&back_models) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.controller, b.controller);
            assert_eq!(a.start_channel_raw, b.start_channel_raw);
            assert_eq!(a.pixel_count, b.pixel_count);
            assert_eq!(a.string_type, b.string_type);
            assert_eq!((a.world_x, a.world_y, a.world_z), (b.world_x, b.world_y, b.world_z));
            assert_eq!((a.x2, a.y2, a.z2), (b.x2, b.y2, b.z2));
        }
        assert_eq!(back_groups.len(), groups.len());
        assert_eq!(back_groups[0].members, groups[0].members);
    }

    #[test]
    fn exported_rig_passes_the_import_gate() {
        let plan = mini_rig();
        let (models, groups) = rig_to_xmodels(&plan, &["robô led 1", "robô led 2"]);
        let xml = export_rgbeffects(&models, &groups);
        let report = import_strings(&networks_for(&["robô led 1", "robô led 2"], 4), &xml);

        assert!(report.conflicts.is_empty(), "we never emit what we would refuse");
        let assigns = report.assignments().expect("gate passes");
        assert_eq!(assigns.len() as u32, plan.total_pixels(), "every pixel mapped");
        // Channel math round-trips: strip 2 of instance 1 starts at abs 61
        // (20px × 3ch after strip 1).
        assert_eq!(models[1].start_channel_raw, "!robô led 1:61");
    }

    #[test]
    fn special_characters_survive_the_roundtrip() {
        let mut m = crate::XModel {
            name: "R&B \"x\" <robô>".into(),
            display_as: "Single Line".into(),
            controller: "ctrl & co".into(),
            start_channel_raw: "!ctrl & co:1".into(),
            pixel_count: 5,
            string_type: "RGB Nodes".into(),
            world_x: 1.0, world_y: 2.0, world_z: 3.0,
            x2: 4.0, y2: 5.0, z2: 6.0,
        };
        let xml = export_rgbeffects(std::slice::from_ref(&m), &[]);
        let (back, _) = parse_rgbeffects(&xml);
        assert_eq!(back[0].name, m.name, "entities must decode back to the original");
        assert_eq!(back[0].controller, m.controller);

        m.name = "plain".into();
        let xml2 = export_rgbeffects(std::slice::from_ref(&m), &[]);
        assert!(!xml2.contains("&amp;amp;"), "no double-escaping");
    }

    #[test]
    fn negative_control_tampered_export_is_caught_by_the_gate() {
        // Corrupt the exported file the way the user's real project was broken
        // (duplicate StartChannel) — the import gate MUST refuse it. If this
        // test ever passes with 0 conflicts, the gate has regressed.
        let (models, groups) = rig_to_xmodels(&mini_rig(), &["robô led 1", "robô led 2"]);
        let xml = export_rgbeffects(&models, &groups);
        let tampered = xml.replacen("!robô led 1:61", "!robô led 1:1", 1);
        assert_ne!(xml, tampered, "tamper must have applied");

        let (bad_models, _) = parse_rgbeffects(&tampered);
        let ctrls = crate::parse_networks(&networks_for(&["robô led 1", "robô led 2"], 4));
        let conflicts = find_channel_conflicts(&bad_models, &ctrls);
        assert!(!conflicts.is_empty(), "gate must catch the duplicated channel");
    }
}
