//! `.lumyx` → [`ShowDescriptor`].
//!
//! ## O que NÃO se faz aqui, e porquê
//!
//! **Não se chama `collect_all`.** O cabeçalho do `.lumyx` traz `pixel_count` mas o
//! `frame_count` pode vir a zero (escritores não-*seekable*), e a duração não está no
//! cabeçalho de todo — sai do carimbo do último quadro. Isso obriga a percorrer o ficheiro.
//!
//! Percorrer **em fluxo**, um quadro de cada vez. Carregar tudo em RAM foi exatamente o
//! problema que a fatia F2 do wearable nomeou: `robot_sequence.lumyx` são **73 MB**, e o pico
//! de memória de um scan não pode depender da duração do show.
//!
//! **Não se verifica integridade.** `pixel_hash` recebe `&[ShowRecord]` — ou seja, exige o
//! show inteiro em memória. Um hash em fluxo não existe hoje. Ver [`Integrity`].

use led_daemon::{ShowDescriptor, ShowId};
use led_show_recorder::{ReadError, ShowReader};

/// O que se sabe sobre a integridade do artefato.
///
/// Existe para que "não verificado" seja **um valor**, e não a ausência de um. Um `bool`
/// deixaria "assumido" e "verificado" indistinguíveis no journal — e é essa distinção que
/// impede o gate de pré-voo de virar carimbo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Integrity {
    /// O operador **afirmou** que o artefato está íntegro (`--assume-integrity`).
    /// Isto **não é verificação**, e o journal regista-o como afirmação.
    AssumedByOperator,
    /// Nada foi verificado nem afirmado.
    NotVerified,
}

impl Integrity {
    /// Vale como `integrity_verified` do pré-voo?
    ///
    /// `AssumedByOperator` vale — mas o journal diz que foi afirmado, não medido. É a mesma
    /// disciplina do ADR-0018: o componente declara, a camada com contexto decide.
    pub fn satisfies_preflight(self) -> bool {
        matches!(self, Integrity::AssumedByOperator)
    }
}

/// Erro ao carregar o descritor.
#[derive(Debug)]
pub enum LoadError {
    Read(ReadError),
    /// Ficheiro sem nenhum quadro — não há show para tocar.
    Empty,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Read(e) => write!(f, "{e}"),
            LoadError::Empty => write!(f, "o ficheiro não tem quadros"),
        }
    }
}

impl From<ReadError> for LoadError {
    fn from(e: ReadError) -> Self {
        LoadError::Read(e)
    }
}

/// Percorre o ficheiro **em fluxo** e devolve o descritor.
///
/// `id` é atribuído por quem chama — o runtime trata-o como opaco (ADR-0023).
pub fn descriptor_from_reader<R: std::io::Read>(
    mut reader: ShowReader<R>,
    id: ShowId,
) -> Result<ShowDescriptor, LoadError> {
    let pixel_count = reader.pixel_count;
    let mut frame_count: u64 = 0;
    let mut last_ts: u64 = 0;

    // Um quadro de cada vez: o pico de memória é UM quadro, não o show.
    while let Some(rec) = reader.next_frame()? {
        frame_count += 1;
        // `max` e não "o último": um ficheiro com carimbos fora de ordem não deve produzir
        // uma duração menor que o instante mais tardio que ele contém.
        last_ts = last_ts.max(rec.timestamp_ms);
    }

    if frame_count == 0 {
        return Err(LoadError::Empty);
    }

    Ok(ShowDescriptor { id, frame_count, pixel_count, duration_ms: last_ts })
}

/// Conveniência para o caminho de ficheiro.
pub fn descriptor_from_path(path: &str, id: ShowId) -> Result<ShowDescriptor, LoadError> {
    let reader = ShowReader::open(path)?;
    descriptor_from_reader(reader, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use led_core::PixelColor;
    use led_show_recorder::{ShowRecord, ShowWriter};

    fn escrever(frames: &[(u64, u8)], pixel_count: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = ShowWriter::new(&mut buf, pixel_count).unwrap();
            for &(ts, v) in frames {
                w.write_frame(&ShowRecord {
                    timestamp_ms: ts,
                    pixels: vec![PixelColor { r: v, g: v, b: v }; pixel_count as usize],
                    audio: None,
                })
                .unwrap();
            }
            w.flush().unwrap();
        }
        buf
    }

    #[test]
    fn descritor_sai_do_ficheiro() {
        let buf = escrever(&[(0, 1), (25, 2), (50, 3)], 4);
        let d = descriptor_from_reader(ShowReader::new(&buf[..]).unwrap(), ShowId(9)).unwrap();
        assert_eq!(d.id, ShowId(9));
        assert_eq!(d.frame_count, 3);
        assert_eq!(d.pixel_count, 4);
        assert_eq!(d.duration_ms, 50, "duração é o carimbo mais tardio");
    }

    /// O cabeçalho traz `frame_count = 0` para escritores não-*seekable*. Se o loader
    /// confiasse nele em vez de contar, esta seria a linha que mentia.
    #[test]
    fn nao_confia_no_frame_count_do_cabecalho() {
        let buf = escrever(&[(0, 1), (10, 2)], 2);
        let r = ShowReader::new(&buf[..]).unwrap();
        assert_eq!(r.frame_count_hint, 0, "premissa: escritor não-seekable deixa a dica a zero");
        let d = descriptor_from_reader(r, ShowId(1)).unwrap();
        assert_eq!(d.frame_count, 2, "contado, não lido do cabeçalho");
    }

    #[test]
    fn carimbos_fora_de_ordem_nao_encurtam_a_duracao() {
        let buf = escrever(&[(0, 1), (90, 2), (30, 3)], 1);
        let d = descriptor_from_reader(ShowReader::new(&buf[..]).unwrap(), ShowId(1)).unwrap();
        assert_eq!(d.duration_ms, 90, "o máximo, não o último");
    }

    #[test]
    fn ficheiro_sem_quadros_e_recusado() {
        let buf = escrever(&[], 3);
        let e = descriptor_from_reader(ShowReader::new(&buf[..]).unwrap(), ShowId(1));
        assert!(matches!(e, Err(LoadError::Empty)));
    }

    #[test]
    fn integridade_assumida_e_distinta_de_verificada() {
        assert!(Integrity::AssumedByOperator.satisfies_preflight());
        assert!(!Integrity::NotVerified.satisfies_preflight());
        // O tipo tem de conseguir DIZER que foi assumido — um `bool` não conseguiria.
        assert_ne!(Integrity::AssumedByOperator, Integrity::NotVerified);
    }
}
