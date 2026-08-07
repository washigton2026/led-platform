//! GS4.2 — a fonte de quadros: `.lumyx` → o frame que corresponde à posição do transporte.
//!
//! ## Em fluxo, com um cursor — nunca o show inteiro em RAM
//!
//! `robot_sequence.lumyx` são **73 MB**. Um `collect_all()` faria o pico de memória depender
//! da duração do show, que foi exatamente o que a fatia F2 do wearable nomeou como errado.
//! Aqui há **um** quadro em memória de cada vez, e o cursor só anda para a frente.
//!
//! ## Seek para trás **reabre o ficheiro**
//!
//! `ShowReader` não sabe recuar. Em vez de guardar tudo para conseguir, o cursor reabre e
//! avança — custa I/O num evento raro (o operador salta), e mantém a memória constante no
//! caso comum (o show a correr). É a troca certa para palco.

use crate::loader::LoadError;
use led_core::{LogicalFrame, PixelColor};
use led_show_recorder::{ShowReader, ShowRecord};

/// Cursor sobre um `.lumyx`, posicionável por instante.
pub struct FrameSource {
    path: String,
    reader: ShowReader<std::fs::File>,
    /// O último quadro lido — o que está "à frente ou em cima" da posição corrente.
    current: Option<ShowRecord>,
    /// O próximo quadro, já espreitado, para saber quando parar de avançar.
    next: Option<ShowRecord>,
    pub pixel_count: u32,
}

impl FrameSource {
    pub fn open(path: &str) -> Result<Self, LoadError> {
        let mut reader = ShowReader::open(path)?;
        let pixel_count = reader.pixel_count;
        let current = reader.next_frame()?;
        let next = reader.next_frame()?;
        Ok(Self { path: path.to_string(), reader, current, next, pixel_count })
    }

    fn reopen(&mut self) -> Result<(), LoadError> {
        let mut reader = ShowReader::open(&self.path)?;
        self.current = reader.next_frame()?;
        self.next = reader.next_frame()?;
        self.reader = reader;
        Ok(())
    }

    /// O quadro cujo carimbo é o **maior ≤ `position_ms`**.
    ///
    /// Devolve `None` só se o ficheiro não tiver quadros. Antes do primeiro carimbo devolve o
    /// primeiro quadro — não há "nada a mostrar" num show que já começou.
    pub fn frame_at(&mut self, position_ms: u64) -> Result<Option<LogicalFrame>, LoadError> {
        // Recuar exige reabrir: o leitor é de sentido único.
        if let Some(c) = &self.current {
            if position_ms < c.timestamp_ms {
                self.reopen()?;
            }
        }
        // Avança enquanto o PRÓXIMO ainda cabe na posição.
        while let Some(n) = &self.next {
            if n.timestamp_ms > position_ms {
                break;
            }
            self.current = self.next.take();
            self.next = self.reader.next_frame()?;
        }
        Ok(self
            .current
            .as_ref()
            .map(|r| LogicalFrame::new(r.pixels.clone(), r.timestamp_ms)))
    }

    /// Um quadro **preto** do tamanho certo, para o caso de o show não ter nada a dizer.
    ///
    /// ⚠️ **Não é blackout.** Isto é o conteúdo de um quadro do show, não uma máscara de
    /// saída; o blackout continua bloqueado pelo ADR-0017 e não existe neste projeto.
    pub fn black(&self) -> LogicalFrame {
        LogicalFrame::new(
            vec![PixelColor { r: 0, g: 0, b: 0 }; self.pixel_count as usize],
            0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use led_show_recorder::ShowWriter;

    fn escrever(nome: &str, frames: &[(u64, u8)], px: u32) -> String {
        let path = std::env::temp_dir().join(nome);
        let f = std::fs::File::create(&path).unwrap();
        let mut w = ShowWriter::new(f, px).unwrap();
        for &(ts, v) in frames {
            w.write_frame(&ShowRecord {
                timestamp_ms: ts,
                pixels: vec![PixelColor { r: v, g: v, b: v }; px as usize],
                audio: None,
            })
            .unwrap();
        }
        w.flush().unwrap();
        path.to_str().unwrap().to_string()
    }

    fn valor(f: &LogicalFrame) -> u8 {
        f.pixels[0].r
    }

    #[test]
    fn devolve_o_quadro_da_posicao() {
        let p = escrever("src_a.lumyx", &[(0, 10), (100, 20), (200, 30)], 3);
        let mut s = FrameSource::open(&p).unwrap();
        assert_eq!(valor(&s.frame_at(0).unwrap().unwrap()), 10);
        assert_eq!(valor(&s.frame_at(99).unwrap().unwrap()), 10, "ainda no 1.º");
        assert_eq!(valor(&s.frame_at(100).unwrap().unwrap()), 20, "a fronteira exata avança");
        assert_eq!(valor(&s.frame_at(150).unwrap().unwrap()), 20);
        assert_eq!(valor(&s.frame_at(9999).unwrap().unwrap()), 30, "depois do fim, o último");
        let _ = std::fs::remove_file(p);
    }

    /// Avanço monótono não reabre o ficheiro — é o caso comum e tem de ser barato.
    #[test]
    fn avanco_para_a_frente_e_um_so_percurso() {
        let p = escrever("src_b.lumyx", &(0..50).map(|i| (i * 10, i as u8)).collect::<Vec<_>>(), 2);
        let mut s = FrameSource::open(&p).unwrap();
        for i in 0..50u64 {
            assert_eq!(valor(&s.frame_at(i * 10).unwrap().unwrap()), i as u8, "t={}", i * 10);
        }
        let _ = std::fs::remove_file(p);
    }

    /// **Seek para trás funciona** — reabrindo. Sem isto, um operador que salta para trás
    /// veria o show congelado no ponto onde estava.
    #[test]
    fn seek_para_tras_reabre_e_acerta() {
        let p = escrever("src_c.lumyx", &[(0, 1), (100, 2), (200, 3), (300, 4)], 2);
        let mut s = FrameSource::open(&p).unwrap();
        assert_eq!(valor(&s.frame_at(300).unwrap().unwrap()), 4);
        assert_eq!(valor(&s.frame_at(0).unwrap().unwrap()), 1, "voltou ao início");
        assert_eq!(valor(&s.frame_at(200).unwrap().unwrap()), 3, "e volta a avançar");
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn ficheiro_sem_quadros_devolve_none() {
        let p = escrever("src_d.lumyx", &[], 4);
        let mut s = FrameSource::open(&p).unwrap();
        assert!(s.frame_at(0).unwrap().is_none());
        assert_eq!(s.black().pixels.len(), 4, "o preto tem o tamanho do rig, não zero");
        let _ = std::fs::remove_file(p);
    }
}
