//! Slice 5 — compilação do profile (ADR-0018).
//!
//! ```text
//! HardwareProfile ──▶ CompiledLayout + DriverConfig ──▶ Runtime
//!        └── depois disto o profile desaparece; nunca é consultado na renderização
//! ```
//!
//! ## Por que isto pode viver no crate leaf
//!
//! [`CompiledLayout`] pertence ao `led-core`, que já é a única dependência deste crate. E a
//! cadeia da governança termina em **"Driver Configuration"**, não em "Driver": instanciar um
//! driver exige socket (I/O = runtime), mas **produzir a configuração é dado**. Logo a
//! compilação não precisa de `led-hal`/`led-protocols`, o crate segue *leaf*, e o `led-hal`
//! permanece intocado — o profile nunca aparece no caminho de render.
//!
//! ## Preset é tipo; endereço é instância
//!
//! [`Identity`](crate::Identity) descreve um **tipo** de hardware (vendor/model/firmware), não
//! um nó específico. Por isso `device_id`, `first_universe` e `address` são parâmetros de
//! **instância** aqui, e não campos do profile.

use std::net::SocketAddr;

use led_core::{
    ColorFormat, CompiledLayout, DeviceId, PixelPhysical, UNIVERSE_SIZE,
};

use crate::{HardwareProfile, OutputInterface, Protocol};

/// Por que uma compilação foi recusada. Recusar é melhor que produzir um layout silenciosamente
/// errado — o palco não perdoa um mapa inválido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// O rig pedido excede o que o nó declara suportar.
    ExceedsMaxPixels { requested: u32, max: u32 },
    /// `pixels_per_universe` é zero — nenhum pixel caberia.
    ZeroPixelsPerUniverse,
    /// Os pixels declarados por universo não cabem em `UNIVERSE_SIZE` com este formato de cor.
    /// O validador (Slice 2) já pega isto; aqui é a defesa de quem compila sem validar antes.
    PixelsExceedUniverse { pixels_per_universe: u16, channels_per_pixel: u16 },
}

/// O que o chamador precisa para **construir** o driver. Dado puro: este crate descreve, o
/// `DeviceDriver` (noutro crate) executa.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriverConfig {
    pub device_id: DeviceId,
    pub protocol: Protocol,
    pub output_interface: OutputInterface,
    /// Endereço do nó — propriedade da **instância**, não do tipo de hardware.
    pub address: SocketAddr,
    pub first_universe: u16,
    pub color: ColorFormat,
    pub pixels_per_universe: u16,
}

/// Compila o profile num [`CompiledLayout`] para `pixel_count` pixels neste nó.
///
/// Honra o `pixels_per_universe` **declarado** — que pode ser menor que o máximo teórico
/// (`UNIVERSE_SIZE / canais`), porque controladores legitimamente empacotam menos. Ignorar o
/// valor declarado esvaziaria o campo.
///
/// Roda no startup (design-time → runtime). Nenhuma alocação no hot-path: o resultado é o
/// artefato apply-once que o HAL consome depois.
pub fn compile_layout(
    profile: &HardwareProfile,
    pixel_count: u32,
    device_id: DeviceId,
    first_universe: u16,
) -> Result<CompiledLayout, CompileError> {
    let ppu = profile.limits.pixels_per_universe;
    if ppu == 0 {
        return Err(CompileError::ZeroPixelsPerUniverse);
    }
    let channels = profile.capabilities.color.channels();
    if ppu as usize * channels > UNIVERSE_SIZE {
        return Err(CompileError::PixelsExceedUniverse {
            pixels_per_universe: ppu,
            channels_per_pixel: channels as u16,
        });
    }
    if pixel_count > profile.limits.max_pixels {
        return Err(CompileError::ExceedsMaxPixels {
            requested: pixel_count,
            max: profile.limits.max_pixels,
        });
    }

    let mut assignments = Vec::with_capacity(pixel_count as usize);
    for i in 0..pixel_count {
        let universe_index = i / ppu as u32;
        let slot = i % ppu as u32;
        assignments.push(PixelPhysical {
            device: device_id,
            universe: first_universe + universe_index as u16,
            channel: (slot as usize * channels) as u16,
            format: profile.capabilities.color,
        });
    }
    Ok(CompiledLayout::compile(&assignments))
}

/// Produz a configuração de driver desta **instância**. Dado puro — nada é aberto, conectado
/// ou enviado aqui.
pub fn driver_config(
    profile: &HardwareProfile,
    device_id: DeviceId,
    address: SocketAddr,
    first_universe: u16,
) -> DriverConfig {
    DriverConfig {
        device_id,
        protocol: profile.capabilities.protocol,
        output_interface: profile.capabilities.output_interface,
        address,
        first_universe,
        color: profile.capabilities.color,
        pixels_per_universe: profile.limits.pixels_per_universe,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HardwareRegistry, RgbOrder, WhiteMode};
    use led_core::{LogicalFrame, PixelColor};

    fn profile(name: &str) -> HardwareProfile {
        HardwareRegistry::with_builtin().profile(name).expect("preset embutido")
    }

    fn addr() -> SocketAddr {
        "192.168.2.156:5568".parse().unwrap()
    }

    #[test]
    fn compiles_a_layout_from_a_builtin_preset() {
        let p = profile("esp32-poe-wled-ddp"); // 170 px/universo, RGB
        let layout = compile_layout(&p, 340, 0, 1).expect("compila");
        assert_eq!(layout.universe_count(), 2, "340 px / 170 = 2 universos");
    }

    /// O ponto da Slice 5: um `pixels_per_universe` MENOR que o máximo teórico é honrado.
    #[test]
    fn a_declared_pixels_per_universe_below_the_maximum_is_honoured() {
        let mut p = profile("esp32-poe-wled-ddp");
        p.limits.pixels_per_universe = 150; // menor que os 170 que caberiam
        let layout = compile_layout(&p, 300, 0, 0).expect("compila");
        assert_eq!(layout.universe_count(), 2, "300 px / 150 = 2 universos (não 300/170)");

        // O pixel 150 é o primeiro do segundo universo → volta ao canal 0.
        let mut scratch = layout.make_scratch();
        let mut px = vec![PixelColor::default(); 300];
        px[150] = PixelColor::rgb(9, 9, 9);
        layout.apply(&LogicalFrame::new(px, 0), &mut scratch);
        assert_eq!(&scratch[1].data[0..3], &[9, 9, 9], "pixel 150 abre o universo seguinte");
    }

    #[test]
    fn rgbw_lays_out_four_channels_per_pixel() {
        let p = profile("generic-sk6812-rgbw-sacn"); // 128 px/universo, RGBW GRB + WhiteMode::Min
        assert_eq!(p.capabilities.color, ColorFormat::Rgbw(RgbOrder::Grb, WhiteMode::MinSubtract));
        let layout = compile_layout(&p, 256, 0, 1).expect("compila");
        assert_eq!(layout.universe_count(), 2, "256 px / 128 = 2 universos");

        let mut scratch = layout.make_scratch();
        let mut px = vec![PixelColor::default(); 256];
        px[1] = PixelColor::rgb(10, 20, 30);
        layout.apply(&LogicalFrame::new(px, 0), &mut scratch);
        // MinSubtract (ADR-0020): W=10, resíduo (0,10,20), ordem GRB -> [10, 0, 20].
        assert_eq!(&scratch[0].data[4..8], &[10, 0, 20, 10], "pixel 1 começa no canal 4");
    }

    #[test]
    fn first_universe_offsets_the_whole_layout() {
        let p = profile("falcon-f16v3-sacn");
        let layout = compile_layout(&p, 170, 7, 100).expect("compila");
        assert_eq!(layout.device_universes(7), &[100], "universo começa onde a instância pede");
    }

    #[test]
    fn asking_for_more_pixels_than_the_node_supports_is_refused() {
        let p = profile("custom"); // max_pixels 1_024
        // `CompiledLayout` é Frozen e não implementa Debug — casamos o padrão em vez de
        // `unwrap_err()`, que exigiria Debug no lado Ok. Nada no led-core é alterado.
        match compile_layout(&p, 2_000, 0, 0) {
            Err(e) => assert_eq!(e, CompileError::ExceedsMaxPixels { requested: 2_000, max: 1_024 }),
            Ok(_) => panic!("pedir mais pixels que o nó suporta deve ser recusado"),
        }
    }

    #[test]
    fn an_impossible_universe_packing_is_refused() {
        let mut p = profile("custom");
        p.limits.pixels_per_universe = 200; // 200 * 3 = 600 > 512
        match compile_layout(&p, 10, 0, 0) {
            Err(e) => assert_eq!(
                e,
                CompileError::PixelsExceedUniverse { pixels_per_universe: 200, channels_per_pixel: 3 }
            ),
            Ok(_) => panic!("empacotamento impossível deve ser recusado"),
        }

        p.limits.pixels_per_universe = 0;
        match compile_layout(&p, 10, 0, 0) {
            Err(e) => assert_eq!(e, CompileError::ZeroPixelsPerUniverse),
            Ok(_) => panic!("zero pixels por universo deve ser recusado"),
        }
    }

    #[test]
    fn driver_config_carries_the_declared_capabilities_and_the_instance_address() {
        let p = profile("esp32-poe-wled-ddp");
        let cfg = driver_config(&p, 3, addr(), 1);
        assert_eq!(cfg.protocol, Protocol::Ddp);
        assert_eq!(cfg.output_interface, OutputInterface::Ethernet);
        assert_eq!(cfg.address, addr(), "endereço é da instância, não do preset");
        assert_eq!(cfg.device_id, 3);
        assert_eq!(cfg.pixels_per_universe, p.limits.pixels_per_universe);
    }

    /// Dois nós do MESMO preset diferem só pelos parâmetros de instância — o preset descreve
    /// um tipo de hardware, não um nó.
    #[test]
    fn two_nodes_of_the_same_preset_differ_only_by_instance_parameters() {
        let p = profile("esp32-poe-wled-ddp");
        let a = driver_config(&p, 0, "192.168.2.156:4048".parse().unwrap(), 0);
        let b = driver_config(&p, 1, "192.168.2.157:4048".parse().unwrap(), 28);
        assert_eq!(a.protocol, b.protocol);
        assert_eq!(a.color, b.color);
        assert_ne!(a.address, b.address);
        assert_ne!(a.first_universe, b.first_universe);
    }
}
