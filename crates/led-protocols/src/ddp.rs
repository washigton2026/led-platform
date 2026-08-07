//! DDP — Distributed Display Protocol
//!
//! A lightweight pixel-streaming protocol, primarily used by WLED in advanced mode.
//! Much simpler than sACN/Art-Net: no universes, no source conflict, no priority.
//! Sends raw RGB pixel data over UDP with a byte offset so a single controller can
//! handle multiple pixel segments at different offsets.
//!
//! ## Wire format (header = 10 bytes, little-endian)
//!
//! ```text
//! byte 0   — flags1  : 0x41 = VER1 | PUSH | REPLY(0) | QUERY(0) | WRITE | STORAGE
//! byte 1   — flags2  : 0x00 (reserved)
//! byte 2   — sequence: monotonic wrapping byte (0..=255)
//! byte 3   — data_type: 0x01 = RGB u8 per channel
//! byte 4-7 — offset  : big-endian u32 — byte offset into the destination buffer
//! byte 8-9 — length  : big-endian u16 — payload byte count
//! byte 10+ — payload : RGB bytes
//! ```
//!
//! ## Invariants (lumyx-network-architect)
//! - Payload MUST be ≤ 1462 bytes (1472 UDP - 10 header) to stay within MTU 1500.
//!   Callers must fragment by pixel segment; this module rejects oversized payloads.
//! - Sequence number is per-device, wrapping byte — never global.
//! - No fragmentation or reassembly: each packet is self-contained.
//! - Fire-and-forget (UDP) — no ACK, no loss detection.

use std::net::{SocketAddr, UdpSocket};

use led_core::{ColorFormat, PixelColor};

/// Maximum pixels per DDP packet (each pixel = 3 bytes RGB).
/// 1472 UDP payload − 10 DDP header = 1462 bytes; floor(1462/3) = 487 whole pixels.
pub const DDP_MAX_PIXELS: usize = 487;

/// Maximum RGB payload bytes per DDP packet (DDP_MAX_PIXELS × 3; fits within 1462-byte limit).
pub const DDP_MAX_PAYLOAD: usize = DDP_MAX_PIXELS * 3; // 1461

/// DDP data type: 8-bit RGB triples.
///
/// **Nota de evidência:** `0x01` **não** é a codificação publicada do campo (que empacota
/// tipo nos bits 6-4 e bits-por-pixel nos bits 3-0, dando `0x13` para RGB de 8 bits). Mas
/// `0x01` é o valor **validado contra hardware real** — WLED 16.0.1, 94/94 frames em
/// 2026-07-20 (`docs/certification/HARDWARE-VALIDATION-2026-07-20.md`). Na prática o WLED
/// infere o formato pelo tamanho do payload. **Não alterar sem re-validar no rig.**
const DDP_DTYPE_RGB8: u8 = 0x01;

/// DDP data type para RGBW de 8 bits, seguindo a codificação publicada
/// (tipo RGBW = `0b011` nos bits 6-4, 8 bits/canal = `0b0011` nos bits 3-0).
///
/// ⚠️ **Não validado em hardware.** Diferente do RGB acima, nenhum controlador confirmou
/// este valor ainda. Como o WLED infere o formato pelo tamanho do payload, o valor tende a
/// ser irrelevante para ele — mas um receptor estrito pode discordar. Item de validação
/// quando houver fita RGBW no rig.
const DDP_DTYPE_RGBW8: u8 = 0x33;

/// DDP flags1: VER1(0x40) | PUSH(0x01) — "this is data, push it immediately".
const DDP_FLAGS1: u8 = 0x41;

/// DDP port (default).
pub const DDP_PORT: u16 = 4048;

/// Bytes de payload que cabem num datagrama DDP (1472 UDP − 10 de cabeçalho).
pub const DDP_MAX_PAYLOAD_BYTES: usize = 1462;

/// Quantos pixels inteiros cabem num pacote para este formato de cor.
/// RGB (3 canais) → 487; RGBW (4 canais) → 365. É por isso que a fragmentação não pode
/// usar a constante de RGB quando o formato é RGBW.
#[inline]
pub fn max_pixels_per_packet(format: ColorFormat) -> usize {
    DDP_MAX_PAYLOAD_BYTES / format.channels()
}

// ── Packet builder ─────────────────────────────────────────────────────────────

/// Build a DDP data packet into `buf`. Returns the total byte length written.
///
/// `offset_bytes` is the byte offset in the destination's pixel buffer.
/// `pixels` must not be empty and must have at most [`DDP_MAX_PIXELS`] elements.
///
/// # Panics
/// Panics if `pixels.len() > DDP_MAX_PIXELS` or `buf` is too short.
pub fn build_ddp_packet(
    buf:          &mut [u8],
    seq:          u8,
    offset_bytes: u32,
    pixels:       &[PixelColor],
) -> usize {
    assert!(!pixels.is_empty(), "DDP: pixels must not be empty");
    assert!(
        pixels.len() <= DDP_MAX_PIXELS,
        "DDP: {} pixels exceeds max {} per packet",
        pixels.len(),
        DDP_MAX_PIXELS
    );
    let payload_len = pixels.len() * 3;
    let total = 10 + payload_len;
    assert!(buf.len() >= total, "DDP: buffer too small ({} < {})", buf.len(), total);

    buf[0] = DDP_FLAGS1;
    buf[1] = 0x00; // flags2 reserved
    buf[2] = seq;
    buf[3] = DDP_DTYPE_RGB8;
    // offset: big-endian u32
    buf[4] = (offset_bytes >> 24) as u8;
    buf[5] = (offset_bytes >> 16) as u8;
    buf[6] = (offset_bytes >>  8) as u8;
    buf[7] =  offset_bytes        as u8;
    // length: big-endian u16
    buf[8]  = (payload_len >> 8) as u8;
    buf[9]  =  payload_len       as u8;
    // payload
    for (i, px) in pixels.iter().enumerate() {
        let b = 10 + i * 3;
        buf[b]     = px.r;
        buf[b + 1] = px.g;
        buf[b + 2] = px.b;
    }
    total
}

/// Build a DDP packet honouring a [`ColorFormat`] — o caminho **pixel-nativo RGBW**.
///
/// Delega a escrita de cada pixel a [`ColorFormat::write`] (ADR-0011), de modo que a
/// derivação do canal branco é **a mesma** usada pelo mapper: não existe segunda
/// implementação de RGBW no projeto.
///
/// O byte de data type acompanha o formato: RGB usa o valor validado em hardware, RGBW usa a
/// codificação publicada (ainda **não** validada em rig — ver [`DDP_DTYPE_RGBW8`]).
///
/// # Panics
/// Se `pixels` estiver vazio, exceder [`max_pixels_per_packet`] ou `buf` for curto demais.
pub fn build_ddp_packet_format(
    buf:          &mut [u8],
    seq:          u8,
    offset_bytes: u32,
    pixels:       &[PixelColor],
    format:       ColorFormat,
) -> usize {
    let channels = format.channels();
    let max_px = max_pixels_per_packet(format);
    assert!(!pixels.is_empty(), "DDP: pixels must not be empty");
    assert!(
        pixels.len() <= max_px,
        "DDP: {} pixels exceeds max {} per packet for {:?}",
        pixels.len(),
        max_px,
        format
    );
    let payload_len = pixels.len() * channels;
    let total = 10 + payload_len;
    assert!(buf.len() >= total, "DDP: buffer too small ({} < {})", buf.len(), total);

    buf[0] = DDP_FLAGS1;
    buf[1] = 0x00;
    buf[2] = seq;
    buf[3] = match format {
        ColorFormat::Rgb(_) => DDP_DTYPE_RGB8,
        ColorFormat::Rgbw(_, _) => DDP_DTYPE_RGBW8,
    };
    buf[4] = (offset_bytes >> 24) as u8;
    buf[5] = (offset_bytes >> 16) as u8;
    buf[6] = (offset_bytes >>  8) as u8;
    buf[7] =  offset_bytes        as u8;
    buf[8] = (payload_len >> 8) as u8;
    buf[9] =  payload_len       as u8;
    for (i, px) in pixels.iter().enumerate() {
        let b = 10 + i * channels;
        format.write(*px, &mut buf[b..b + channels]);
    }
    total
}

/// Build a DDP data packet from a **raw RGB byte payload already in wire order**.
///
/// The zero-copy sibling of [`build_ddp_packet`]: the payload is written with a single
/// `copy_from_slice` instead of iterating `PixelColor`s. A caller that already holds mapped
/// channel bytes (a `DeviceDriver` on the send path) can therefore build a packet without
/// allocating an intermediate `Vec<PixelColor>` — the hot-path allocation this replaced.
///
/// `payload` must be non-empty and at most [`DDP_MAX_PAYLOAD`] bytes. Returns the total
/// byte length written.
///
/// # Panics
/// Panics if `payload` is empty, exceeds [`DDP_MAX_PAYLOAD`], or `buf` is too short.
pub fn build_ddp_packet_bytes(
    buf:          &mut [u8],
    seq:          u8,
    offset_bytes: u32,
    payload:      &[u8],
) -> usize {
    assert!(!payload.is_empty(), "DDP: payload must not be empty");
    assert!(
        payload.len() <= DDP_MAX_PAYLOAD,
        "DDP: {} bytes exceeds max {} per packet",
        payload.len(),
        DDP_MAX_PAYLOAD
    );
    let total = 10 + payload.len();
    assert!(buf.len() >= total, "DDP: buffer too small ({} < {})", buf.len(), total);

    buf[0] = DDP_FLAGS1;
    buf[1] = 0x00; // flags2 reserved
    buf[2] = seq;
    buf[3] = DDP_DTYPE_RGB8;
    // offset: big-endian u32
    buf[4] = (offset_bytes >> 24) as u8;
    buf[5] = (offset_bytes >> 16) as u8;
    buf[6] = (offset_bytes >>  8) as u8;
    buf[7] =  offset_bytes        as u8;
    // length: big-endian u16
    buf[8] = (payload.len() >> 8) as u8;
    buf[9] =  payload.len()       as u8;
    // payload — single memcpy, no per-pixel iteration, no allocation
    buf[10..total].copy_from_slice(payload);
    total
}

/// Parse a DDP packet from `raw`. Returns `None` for malformed packets.
pub fn parse_ddp_packet(raw: &[u8]) -> Option<DdpPacket<'_>> {
    if raw.len() < 10 { return None; }
    // Validate VER1 bit (bits 7-6 of flags1 must be 01)
    if raw[0] & 0xC0 != 0x40 { return None; }
    let seq  = raw[2];
    let dtype = raw[3];
    let offset_bytes =
        ((raw[4] as u32) << 24) | ((raw[5] as u32) << 16) |
        ((raw[6] as u32) <<  8) |  (raw[7] as u32);
    let length =
        ((raw[8] as usize) << 8) | (raw[9] as usize);
    if raw.len() < 10 + length { return None; }
    Some(DdpPacket { seq, dtype, offset_bytes, payload: &raw[10..10 + length] })
}

/// Parsed DDP packet (zero-copy view into the source buffer).
#[derive(Debug, PartialEq, Eq)]
pub struct DdpPacket<'a> {
    pub seq:          u8,
    pub dtype:        u8,
    pub offset_bytes: u32,
    pub payload:      &'a [u8],
}

// ── DdpDevice ─────────────────────────────────────────────────────────────────

/// A DDP output device. Sends pixel data over UDP to a single target address.
///
/// Each DDP device is responsible for one logical segment of pixels. If the segment
/// exceeds [`DDP_MAX_PIXELS`], `send_pixels` automatically fragments into multiple packets.
///
/// # Sequence number
/// Per-device, wrapping byte — this is the canonical LUMYX invariant for DDP.
pub struct DdpDevice {
    socket:     UdpSocket,
    target:     SocketAddr,
    seq:        u8,
    /// Formato de cor por pixel. `Rgb(Rgb)` por padrão — byte-idêntico ao comportamento
    /// anterior ao suporte RGBW.
    format:     ColorFormat,
    /// Pre-allocated send buffer (10 + DDP_MAX_PAYLOAD bytes).
    buf:        Box<[u8; 10 + DDP_MAX_PAYLOAD]>,
    /// Byte offset in the destination's pixel buffer where this segment starts.
    pub pixel_offset: u32,
    /// Teto de pixels por datagrama. `None` = o máximo que a MTU Ethernet padrão permite.
    ///
    /// Existe porque a MTU **não é uma constante do protocolo**: é uma propriedade do caminho,
    /// e um `HardwareProfile` pode declarar outra (VPN, PPPoE, jumbo). Sem isto, um MTU
    /// declarado seria um número que ninguém honra — pior que não o declarar.
    max_px_override: Option<usize>,
}

impl std::fmt::Debug for DdpDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DdpDevice")
            .field("target", &self.target)
            .field("seq", &self.seq)
            .field("pixel_offset", &self.pixel_offset)
            .finish()
    }
}

impl DdpDevice {
    /// Create a new DDP device targeting `addr`. `pixel_offset` is the number of
    /// pixels (not bytes) before this segment in the destination's pixel buffer.
    pub fn new(addr: SocketAddr, pixel_offset: u32) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(addr)?;
        Ok(Self {
            socket,
            target: addr,
            seq: 0,
            format: ColorFormat::Rgb(led_core::RgbOrder::Rgb),
            buf: Box::new([0u8; 10 + DDP_MAX_PAYLOAD]),
            pixel_offset,
            max_px_override: None,
        })
    }

    /// Cria um device com um [`ColorFormat`] explícito — é assim que o caminho pixel-nativo
    /// emite RGBW (ADR-0011). A fragmentação passa a usar [`max_pixels_per_packet`], porque
    /// RGBW cabe 365 px/pacote e não 487.
    pub fn with_format(
        addr: SocketAddr,
        pixel_offset: u32,
        format: ColorFormat,
    ) -> std::io::Result<Self> {
        let mut d = Self::new(addr, pixel_offset)?;
        d.format = format;
        Ok(d)
    }

    /// O formato de cor deste device.
    pub fn format(&self) -> ColorFormat {
        self.format
    }

    /// Limita os pixels por datagrama — tipicamente derivado do MTU declarado por um
    /// `HardwareProfile`. **Nunca aumenta** acima do que o buffer e a MTU padrão comportam:
    /// um profile com MTU maior que a rede real produziria datagramas que se perdem.
    pub fn set_max_pixels(&mut self, n: usize) {
        self.max_px_override = Some(n.max(1));
    }

    /// O teto efetivo de pixels por datagrama, já com o formato de cor em conta.
    pub fn max_pixels(&self) -> usize {
        let teto = max_pixels_per_packet(self.format);
        self.max_px_override.map_or(teto, |n| n.min(teto))
    }

    /// Send `pixels` to the device, automatically fragmenting if needed.
    /// Each fragment's byte offset is derived from `pixel_offset + fragment_start`.
    pub fn send_pixels(&mut self, pixels: &[PixelColor]) -> std::io::Result<()> {
        let channels = self.format.channels();
        let max_px = self.max_pixels();
        let mut sent = 0usize;
        while sent < pixels.len() {
            let chunk_end = (sent + max_px).min(pixels.len());
            let chunk = &pixels[sent..chunk_end];
            let byte_offset = ((self.pixel_offset as usize + sent) * channels) as u32;
            let len = build_ddp_packet_format(
                self.buf.as_mut(),
                self.seq,
                byte_offset,
                chunk,
                self.format,
            );
            self.socket.send(&self.buf[..len])?;
            self.seq = self.seq.wrapping_add(1);
            sent = chunk_end;
        }
        Ok(())
    }

    /// Current sequence counter (for inspection/testing).
    pub fn seq(&self) -> u8 { self.seq }

    /// Target address.
    pub fn target(&self) -> SocketAddr { self.target }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::{ColorFormat, PixelColor};

    fn px(r: u8, g: u8, b: u8) -> PixelColor { PixelColor::rgb(r, g, b) }

    // ── Wire format ───────────────────────────────────────────────────────────

    // ── RGBW pixel-nativo (A2) ────────────────────────────────────────────────

    #[test]
    fn packet_capacity_depends_on_the_colour_format() {
        use led_core::{RgbOrder, WhiteMode};
        assert_eq!(max_pixels_per_packet(ColorFormat::Rgb(RgbOrder::Rgb)), 487, "1462/3");
        assert_eq!(
            max_pixels_per_packet(ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min)),
            365,
            "1462/4 — RGBW cabe menos pixels por datagrama"
        );
    }

    #[test]
    fn rgbw_packet_carries_four_channels_with_the_white_derived_by_the_contract() {
        use led_core::{RgbOrder, WhiteMode};
        let fmt = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min);
        let mut buf = [0u8; 64];
        let px = [px(10, 20, 30)];
        let len = build_ddp_packet_format(&mut buf, 0, 0, &px, fmt);
        assert_eq!(len, 14, "10 de cabeçalho + 4 canais");
        // GRB + W = min(10,20,30) = 10 — mesma derivação do mapper (ADR-0011).
        assert_eq!(&buf[10..14], &[20, 10, 30, 10]);
    }

    #[test]
    fn the_data_type_byte_follows_the_colour_format() {
        use led_core::{RgbOrder, WhiteMode};
        let mut buf = [0u8; 64];
        let px = [px(1, 2, 3)];
        build_ddp_packet_format(&mut buf, 0, 0, &px, ColorFormat::Rgb(RgbOrder::Rgb));
        assert_eq!(buf[3], DDP_DTYPE_RGB8, "RGB mantém o valor validado em hardware");
        build_ddp_packet_format(&mut buf, 0, 0, &px, ColorFormat::Rgbw(RgbOrder::Rgb, WhiteMode::None));
        assert_eq!(buf[3], DDP_DTYPE_RGBW8, "RGBW usa a codificação publicada (não validada em rig)");
    }

    /// Retrocompatibilidade: o builder de formato com RGB produz **exatamente** os mesmos
    /// bytes do builder histórico — o caminho validado em hardware não mudou.
    #[test]
    fn rgb_via_format_builder_is_byte_identical_to_the_legacy_builder() {
        use led_core::RgbOrder;
        let pixels: Vec<PixelColor> = (0..50).map(|i| px(i as u8, (i * 2) as u8, (i * 3) as u8)).collect();
        let mut a = [0u8; 10 + DDP_MAX_PAYLOAD];
        let mut b = [0u8; 10 + DDP_MAX_PAYLOAD];
        let la = build_ddp_packet(&mut a, 7, 300, &pixels);
        let lb = build_ddp_packet_format(&mut b, 7, 300, &pixels, ColorFormat::Rgb(RgbOrder::Rgb));
        assert_eq!(la, lb);
        assert_eq!(a[..la], b[..lb], "RGB pelo novo caminho == RGB pelo caminho antigo");
    }

    #[test]
    fn an_rgbw_device_fragments_at_365_pixels() {
        use led_core::{RgbOrder, WhiteMode};
        use std::net::UdpSocket;
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_millis(500))).unwrap();
        let addr = rx.local_addr().unwrap();

        let fmt = ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::Min);
        let mut dev = DdpDevice::with_format(addr, 0, fmt).unwrap();
        assert_eq!(dev.format(), fmt);
        // 400 px > 365 → dois pacotes.
        dev.send_pixels(&vec![px(10, 20, 30); 400]).unwrap();

        let mut buf = [0u8; 2048];
        let n1 = rx.recv(&mut buf).expect("1º pacote");
        let p1 = parse_ddp_packet(&buf[..n1]).expect("parse");
        assert_eq!(p1.payload.len(), 365 * 4, "1º pacote cheio: 365 px RGBW");
        assert_eq!(p1.offset_bytes, 0);
        assert_eq!(p1.dtype, DDP_DTYPE_RGBW8);

        let n2 = rx.recv(&mut buf).expect("2º pacote");
        let p2 = parse_ddp_packet(&buf[..n2]).expect("parse");
        assert_eq!(p2.payload.len(), 35 * 4, "resto: 400-365 = 35 px");
        assert_eq!(p2.offset_bytes, 365 * 4, "offset em BYTES avança por 4 canais/pixel");
    }

    #[test]
    fn ddp_header_flags_and_version() {
        let pixels = vec![px(0xFF, 0x00, 0x00)]; // 1 red pixel
        let mut buf = [0u8; 64];
        let len = build_ddp_packet(&mut buf, 0x00, 0, &pixels);
        assert_eq!(len, 13); // 10 header + 3 payload
        assert_eq!(buf[0], 0x41, "flags1: VER1|PUSH");
        assert_eq!(buf[1], 0x00, "flags2: reserved");
        assert_eq!(buf[2], 0x00, "sequence");
        assert_eq!(buf[3], 0x01, "dtype: RGB8");
    }

    #[test]
    fn ddp_offset_big_endian() {
        let pixels = vec![px(0, 0, 0)];
        let mut buf = [0u8; 64];
        build_ddp_packet(&mut buf, 0, 0x01_02_03_04, &pixels);
        assert_eq!(&buf[4..8], &[0x01, 0x02, 0x03, 0x04], "offset big-endian");
    }

    #[test]
    fn ddp_length_big_endian() {
        let pixels = vec![px(0, 0, 0); 100]; // 300 bytes
        let mut buf = [0u8; 10 + 300];
        build_ddp_packet(&mut buf, 0, 0, &pixels);
        let len_field = ((buf[8] as usize) << 8) | buf[9] as usize;
        assert_eq!(len_field, 300, "length field must be 300 (100 pixels × 3)");
    }

    #[test]
    fn ddp_payload_rgb_order() {
        let pixels = vec![px(0xAA, 0xBB, 0xCC)];
        let mut buf = [0u8; 64];
        let len = build_ddp_packet(&mut buf, 0, 0, &pixels);
        assert_eq!(&buf[10..len], &[0xAA, 0xBB, 0xCC], "RGB order must be R-G-B");
    }

    #[test]
    fn ddp_multi_pixel_payload() {
        let pixels = vec![px(1, 2, 3), px(4, 5, 6), px(7, 8, 9)];
        let mut buf = [0u8; 64];
        let len = build_ddp_packet(&mut buf, 0, 0, &pixels);
        assert_eq!(len, 19); // 10 + 9
        assert_eq!(&buf[10..19], &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    // ── Parse round-trip ─────────────────────────────────────────────────────

    #[test]
    fn ddp_parse_round_trip() {
        let pixels = vec![px(0x10, 0x20, 0x30), px(0x40, 0x50, 0x60)];
        let mut buf = [0u8; 64];
        let len = build_ddp_packet(&mut buf, 0x07, 0x00_00_00_06, &pixels);
        let parsed = parse_ddp_packet(&buf[..len]).expect("must parse");
        assert_eq!(parsed.seq, 0x07);
        assert_eq!(parsed.dtype, DDP_DTYPE_RGB8);
        assert_eq!(parsed.offset_bytes, 6);
        assert_eq!(parsed.payload, &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60]);
    }

    #[test]
    fn ddp_parse_too_short_header() {
        assert!(parse_ddp_packet(&[0x41, 0x00, 0x01]).is_none(), "< 10 bytes → None");
    }

    #[test]
    fn ddp_parse_wrong_version() {
        // flags1 with version != 01 (bits 7-6)
        let mut buf = [0u8; 16];
        buf[0] = 0x80; // version 10 — invalid
        buf[8] = 0x00; buf[9] = 0x03;
        assert!(parse_ddp_packet(&buf[..13]).is_none(), "wrong version → None");
    }

    #[test]
    fn ddp_parse_payload_length_mismatch() {
        let mut buf = [0u8; 16];
        buf[0] = 0x41;
        buf[8] = 0x00; buf[9] = 0xFF; // claims 255 bytes payload
        // but buffer only has 6 bytes after header
        assert!(parse_ddp_packet(&buf[..16]).is_none(), "truncated payload → None");
    }

    // ── Sequence counter ─────────────────────────────────────────────────────

    #[test]
    fn ddp_sequence_wraps_at_255() {
        let mut buf = [0u8; 64];
        let pixels = vec![px(0, 0, 0)];
        // seq 255 → 0 (wrapping)
        build_ddp_packet(&mut buf, 255, 0, &pixels);
        assert_eq!(buf[2], 255);
        build_ddp_packet(&mut buf, 255u8.wrapping_add(1), 0, &pixels);
        assert_eq!(buf[2], 0, "sequence wraps 255→0");
    }

    // ── Constraints ──────────────────────────────────────────────────────────

    #[test]
    fn ddp_max_pixels_constant() {
        // Each DDP_MAX_PIXELS pixels = DDP_MAX_PIXELS * 3 bytes = DDP_MAX_PAYLOAD
        assert_eq!(DDP_MAX_PIXELS * 3, DDP_MAX_PAYLOAD);
        // compile-time invariant: payload must fit within the 1472-10 byte UDP limit
        const _: () = assert!(DDP_MAX_PAYLOAD <= 1462);
    }

    #[test]
    fn ddp_exactly_max_pixels_fits() {
        let pixels = vec![px(0xFF, 0, 0); DDP_MAX_PIXELS];
        let mut buf = vec![0u8; 10 + DDP_MAX_PAYLOAD];
        let len = build_ddp_packet(&mut buf, 0, 0, &pixels);
        assert_eq!(len, 10 + DDP_MAX_PAYLOAD);
    }

    // ── UDP loopback (wire delivery) ──────────────────────────────────────────

    #[test]
    fn ddp_udp_loopback() {
        use std::net::UdpSocket;
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = recv.local_addr().unwrap();
        recv.set_read_timeout(Some(std::time::Duration::from_millis(500))).unwrap();

        let mut device = DdpDevice::new(addr, 0).unwrap();
        let pixels = vec![px(0xDE, 0xAD, 0xBE), px(0xEF, 0x00, 0x01)];
        device.send_pixels(&pixels).unwrap();

        let mut buf = [0u8; 512];
        let n = recv.recv(&mut buf).expect("packet must arrive");
        let parsed = parse_ddp_packet(&buf[..n]).expect("must parse");
        assert_eq!(parsed.payload, &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
        assert_eq!(parsed.offset_bytes, 0);
        assert_eq!(parsed.seq, 0);
        assert_eq!(device.seq(), 1, "seq incremented after send");
    }

    #[test]
    fn ddp_device_pixel_offset_in_wire() {
        use std::net::UdpSocket;
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = recv.local_addr().unwrap();
        recv.set_read_timeout(Some(std::time::Duration::from_millis(500))).unwrap();

        // pixel_offset=100 → byte_offset=300 in wire
        let mut device = DdpDevice::new(addr, 100).unwrap();
        device.send_pixels(&[px(1, 2, 3)]).unwrap();

        let mut buf = [0u8; 512];
        let n = recv.recv(&mut buf).unwrap();
        let parsed = parse_ddp_packet(&buf[..n]).unwrap();
        assert_eq!(parsed.offset_bytes, 300, "pixel 100 → byte offset 300");
    }

    // ── Fragmentation ─────────────────────────────────────────────────────────

    #[test]
    fn ddp_auto_fragment_large_segment() {
        use std::net::UdpSocket;
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = recv.local_addr().unwrap();
        recv.set_read_timeout(Some(std::time::Duration::from_millis(200))).unwrap();

        let total_pixels = DDP_MAX_PIXELS + 10; // forces 2 packets
        let pixels: Vec<PixelColor> = (0..total_pixels)
            .map(|i| px(i as u8, (i >> 8) as u8, 0))
            .collect();

        let mut device = DdpDevice::new(addr, 0).unwrap();
        device.send_pixels(&pixels).unwrap();

        // Collect both packets
        let mut packets_received = 0usize;
        let mut total_pixels_rx = 0usize;
        let mut buf = [0u8; 10 + DDP_MAX_PAYLOAD + 64];
        for _ in 0..2 {
            if let Ok(n) = recv.recv(&mut buf) {
                if let Some(pkt) = parse_ddp_packet(&buf[..n]) {
                    packets_received += 1;
                    total_pixels_rx += pkt.payload.len() / 3;
                }
            }
        }
        assert_eq!(packets_received, 2, "must fragment into exactly 2 packets");
        assert_eq!(total_pixels_rx, total_pixels, "must deliver all pixels");
        assert_eq!(device.seq(), 2, "seq incremented once per packet");
    }

    // ── Adversarial ───────────────────────────────────────────────────────────

    #[test]
    fn ddp_parse_empty_slice() {
        assert!(parse_ddp_packet(&[]).is_none());
    }

    #[test]
    fn ddp_parse_exact_header_no_payload() {
        let buf = [0x41u8, 0, 0, 1, 0, 0, 0, 0, 0, 0]; // length=0
        let parsed = parse_ddp_packet(&buf).expect("zero-length payload is valid");
        assert_eq!(parsed.payload.len(), 0);
    }

    #[test]
    fn ddp_offset_continuity_across_fragments() {
        // Build two consecutive packets and verify offsets are contiguous
        let mut buf1 = vec![0u8; 10 + DDP_MAX_PAYLOAD];
        let mut buf2 = vec![0u8; 64];
        let full = vec![px(0xFF, 0, 0); DDP_MAX_PIXELS];
        let tail = vec![px(0, 0xFF, 0); 5];

        build_ddp_packet(&mut buf1, 0, 0, &full);
        build_ddp_packet(&mut buf2, 1, (DDP_MAX_PIXELS * 3) as u32, &tail);

        let p1 = parse_ddp_packet(&buf1).unwrap();
        let p2 = parse_ddp_packet(&buf2).unwrap();

        // p2.offset_bytes must immediately follow p1's last byte
        assert_eq!(
            p2.offset_bytes,
            p1.offset_bytes + p1.payload.len() as u32,
            "second fragment offset must continue from first"
        );
    }
}
