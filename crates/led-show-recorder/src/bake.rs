//! `bake` — recorta um show do rig no artefato de **um traje** (FASE F2, ADR-0022 D1).
//!
//! ```text
//! show do rig (6.200 px)  ──bake(faixas do traje)──▶  show do traje (N px)
//! ```
//!
//! ## Por que isto é fluxo, e não uma função sobre `Vec<ShowRecord>`
//!
//! O achado que dá forma a este módulo: [`ShowReader::next_frame`](crate::ShowReader::next_frame)
//! **já** transmite quadro a quadro, mas `led_player::play` recebe `&[ShowRecord]` e o binário
//! faz `collect_all()` — o show inteiro em RAM. Para desktop está certo e é decisão
//! documentada (permite verificar o manifesto antes do 1º quadro). Para um traje, **não**:
//! um ESP32 tem ~520 kB de SRAM e um artefato de 4 min × 400 px tem ~11 MB. Não cabe, nem
//! perto.
//!
//! Portanto `bake` nunca materializa o show. Lê um quadro, escreve um quadro, esquece. O
//! pico de memória é **um quadro de origem + um quadro de destino**, independentemente de o
//! show ter 90 segundos ou 90 minutos.
//!
//! ## O artefato do traje é o MESMO formato
//!
//! Não existe "formato de traje". O resultado é um `.lumyx` comum, com menos pixels — logo
//! `ShowReader`, `pixel_hash` e `ReplayManifest` continuam **aplicáveis** sem uma linha nova.
//! Um segundo formato seria uma segunda representação da mesma coisa, que é exatamente o que
//! a governança deste repo proíbe.
//!
//! ## ⚠️ O que o recorte NÃO herda: autenticidade
//!
//! **Uma redação anterior deste módulo sugeriu que "a assinatura Ed25519 continua valendo sem
//! uma linha nova". Isso é verdade sobre o FORMATO e FALSO sobre a SEGURANÇA — está corrigido
//! aqui.**
//!
//! `signing.rs:46-56` assina exatamente
//! `SIGNING_VERSION | frame_count | pixel_count | aggregate_hash | frame_hashes[..]`.
//! No recorte, `pixel_count` muda, `aggregate_hash` muda e **todo** `frame_hashes` muda.
//! Logo a assinatura do show do rig **não autentica o artefato derivado** — não por política,
//! por aritmética.
//!
//! E [`bake`] **não produz** manifesto nem sidecar para o artefato. Enquanto isso não existir
//! (ver **TD-013**), um artefato recortado é **bytes não autenticados**: use apenas em
//! bancada. A cobertura correta é a próxima fatia obrigatória do F2 — mesmo `ReplayManifest`,
//! mesma `ShowSigner`, mesmo `verify_manifest_pinned`, mesmo sidecar (ADR-0004). Nenhuma
//! segunda assinatura, nenhum formato paralelo.
//!
//! ## Quem é o traje é **dado**, não dependência
//!
//! `bake` recebe **faixas de pixels**. Não sabe o que é um grupo do xLights, uma instância
//! de `RigPlan` ou um controlador — quem sabe passa as faixas. Mesma disciplina do ADR-0018
//! ("injeção de dado, não de dependência"): amarrar o `bake` a uma origem de endereçamento
//! escolheria por antecipação uma decisão que ainda não precisa ser tomada.

use std::io::{Read, Seek, Write};
use std::ops::Range;

use crate::{finalise_seekable, ReadError, ShowReader, ShowRecord, ShowWriter};

/// Por que um `bake` foi recusado. Recusar é melhor que produzir um traje silenciosamente
/// errado — um pixel deslocado vira um membro apagado em cena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BakeError {
    /// Nenhuma faixa — o traje não teria pixel nenhum.
    EmptySubset,
    /// Uma faixa é vazia ou invertida (`start >= end`).
    DegenerateRange { index: usize, start: usize, end: usize },
    /// Uma faixa passa do fim do show de origem. **Nunca truncamos em silêncio:** truncar
    /// produziria um traje com menos pixels que o esperado e o erro só apareceria no palco.
    OutOfBounds { index: usize, end: usize, source_pixels: u32 },
    /// Duas faixas cobrem o mesmo pixel de origem.
    ///
    /// **Recusado de propósito.** Duplicar um pixel é quase sempre erro de montagem (faixas
    /// coladas à mão, grupo aninhado contado duas vezes) e o sintoma — dois trechos do traje
    /// acendendo juntos — é difícil de atribuir no palco. Se um dia houver caso legítimo
    /// (espelhar um segmento), ele ganha uma API própria e explícita, não um silêncio aqui.
    OverlappingRanges { a: usize, b: usize, pixel: usize },
    Read(String),
    Write(String),
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BakeError::EmptySubset => write!(f, "subset vazio: o traje não teria pixels"),
            BakeError::DegenerateRange { index, start, end } => {
                write!(f, "faixa {index} degenerada: {start}..{end}")
            }
            BakeError::OutOfBounds { index, end, source_pixels } => write!(
                f,
                "faixa {index} termina em {end}, mas o show tem {source_pixels} pixels"
            ),
            BakeError::OverlappingRanges { a, b, pixel } => write!(
                f,
                "faixas {a} e {b} cobrem o mesmo pixel {pixel} — duplicação silenciosa é proibida"
            ),
            BakeError::Read(e) => write!(f, "leitura: {e}"),
            BakeError::Write(e) => write!(f, "escrita: {e}"),
        }
    }
}

impl std::error::Error for BakeError {}

/// Quantos pixels um conjunto de faixas produz. Útil para dimensionar antes de assar.
pub fn subset_len(ranges: &[Range<usize>]) -> usize {
    ranges.iter().map(|r| r.end.saturating_sub(r.start)).sum()
}

/// Valida as faixas contra um show de `source_pixels` pixels, sem ler nada.
pub fn validate_subset(ranges: &[Range<usize>], source_pixels: u32) -> Result<(), BakeError> {
    if ranges.is_empty() {
        return Err(BakeError::EmptySubset);
    }
    for (i, r) in ranges.iter().enumerate() {
        if r.start >= r.end {
            return Err(BakeError::DegenerateRange { index: i, start: r.start, end: r.end });
        }
        if r.end > source_pixels as usize {
            return Err(BakeError::OutOfBounds { index: i, end: r.end, source_pixels });
        }
    }
    // Sobreposição: O(n²) sobre as FAIXAS (não sobre os pixels) e roda uma vez, no assar.
    for (i, a) in ranges.iter().enumerate() {
        for (j, b) in ranges.iter().enumerate().skip(i + 1) {
            let lo = a.start.max(b.start);
            let hi = a.end.min(b.end);
            if lo < hi {
                return Err(BakeError::OverlappingRanges { a: i, b: j, pixel: lo });
            }
        }
    }
    Ok(())
}

/// Recorta `reader` nas `ranges` e escreve o artefato do traje em `out`.
///
/// **Fluxo puro:** um quadro de cada vez. O pico de memória não depende da duração do show.
///
/// A ordem dos pixels no traje é a ordem das faixas, concatenadas — `ranges` é a definição
/// de "qual é este traje", e a ordem faz parte dessa definição.
///
/// Devolve quantos quadros foram assados.
pub fn bake<R: Read, W: Write + Seek>(
    reader: ShowReader<R>,
    ranges: &[Range<usize>],
    out: W,
) -> Result<u32, BakeError> {
    let source_pixels = reader.pixel_count;
    validate_subset(ranges, source_pixels)?;

    let n = subset_len(ranges);
    let mut writer =
        ShowWriter::new(out, n as u32).map_err(|e| BakeError::Write(e.to_string()))?;

    // Os dois únicos buffers vivos: o quadro lido e o quadro recortado.
    let mut sub = ShowRecord { timestamp_ms: 0, pixels: vec![Default::default(); n], audio: None };

    let mut reader = reader;
    let mut frames = 0u32;
    loop {
        let rec = match reader.next_frame() {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => return Err(BakeError::Read(read_err(&e))),
        };

        let mut w = 0usize;
        for r in ranges {
            let take = r.end - r.start;
            sub.pixels[w..w + take].copy_from_slice(&rec.pixels[r.start..r.end]);
            w += take;
        }
        sub.timestamp_ms = rec.timestamp_ms;
        // O instantâneo de áudio é metadado de análise e viaja intacto: preservá-lo mantém
        // `ReplayManifest` e a verificação de integridade com a mesma semântica do original.
        sub.audio = rec.audio;

        writer.write_frame(&sub).map_err(|e| BakeError::Write(e.to_string()))?;
        frames += 1;
    }

    finalise_seekable(&mut writer).map_err(|e| BakeError::Write(e.to_string()))?;
    Ok(frames)
}

fn read_err(e: &ReadError) -> String {
    format!("{e:?}")
}

/// Tamanho exato, em bytes, de um artefato — **derivado do formato, não estimado**.
///
/// Cabeçalho de 16 B + por quadro: timestamp `u64` (8 B) + `pixels × 3 B` + flag de áudio
/// (1 B). Verificado ao byte contra `robot_sequence.lumyx`: 6.200 px × 3.925 quadros =
/// 73 005 000 B crus + 35 341 B de overhead = 73 040 341 B, que é o tamanho real do arquivo.
///
/// Existe para que o dimensionamento de flash de um traje seja **calculado antes** de assar,
/// e não descoberto quando o gravador falha.
pub const fn artifact_bytes(pixels: u32, frames: u32) -> u64 {
    16 + (frames as u64) * (8 + (pixels as u64) * 3 + 1)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel_hash;
    use led_core::PixelColor;
    use std::io::Cursor;

    /// Show sintético em que a cor codifica o índice do pixel — assim qualquer troca de
    /// posição no recorte é detectável por igualdade exata, não por "parece certo".
    fn source(px: usize, frames: u64) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = ShowWriter::new(&mut buf, px as u32).unwrap();
            for f in 0..frames {
                let pixels = (0..px)
                    .map(|i| PixelColor::rgb((i % 256) as u8, (i / 256) as u8, f as u8))
                    .collect();
                w.write_frame(&ShowRecord { timestamp_ms: f * 25, pixels, audio: None }).unwrap();
            }
            finalise_seekable(&mut w).unwrap();
        }
        buf.into_inner()
    }

    fn bake_to_vec(src: &[u8], ranges: &[Range<usize>]) -> Result<(Vec<u8>, u32), BakeError> {
        let reader = ShowReader::new(Cursor::new(src.to_vec())).unwrap();
        let mut out = Cursor::new(Vec::new());
        let n = bake(reader, ranges, &mut out)?;
        Ok((out.into_inner(), n))
    }

    #[test]
    fn baked_pixels_are_byte_identical_to_the_source_range() {
        let src = source(1000, 5);
        let ranges = vec![100..150, 700..730];
        let (baked, frames) = bake_to_vec(&src, &ranges).expect("assa");
        assert_eq!(frames, 5);

        let orig = ShowReader::new(Cursor::new(src)).unwrap().collect_all().unwrap();
        let out = ShowReader::new(Cursor::new(baked)).unwrap().collect_all().unwrap();
        assert_eq!(out.len(), 5);

        for (o, b) in orig.iter().zip(out.iter()) {
            assert_eq!(o.timestamp_ms, b.timestamp_ms, "timestamp tem que sobreviver");
            assert_eq!(b.pixels.len(), 80, "50 + 30 pixels");
            let expected: Vec<PixelColor> =
                ranges.iter().flat_map(|r| o.pixels[r.clone()].iter().copied()).collect();
            assert_eq!(b.pixels, expected, "os bytes do traje são os bytes do rig");
        }
    }

    #[test]
    fn range_order_defines_the_costume_and_is_preserved() {
        // Faixas fora de ordem crescente são legítimas: a ordem é a definição do traje.
        let src = source(100, 1);
        let (a, _) = bake_to_vec(&src, &[10..12, 50..52]).unwrap();
        let (b, _) = bake_to_vec(&src, &[50..52, 10..12]).unwrap();
        assert_ne!(a, b, "inverter as faixas tem que produzir um traje diferente");

        let ra = ShowReader::new(Cursor::new(a)).unwrap().collect_all().unwrap();
        assert_eq!(ra[0].pixels[0], PixelColor::rgb(10, 0, 0), "1º pixel é o 10 do rig");
        assert_eq!(ra[0].pixels[2], PixelColor::rgb(50, 0, 0), "3º pixel é o 50 do rig");
    }

    #[test]
    fn bake_is_deterministic() {
        let src = source(300, 8);
        let (a, _) = bake_to_vec(&src, std::slice::from_ref(&(0..64))).unwrap();
        let (b, _) = bake_to_vec(&src, std::slice::from_ref(&(0..64))).unwrap();
        assert_eq!(a, b, "mesmo show + mesmas faixas ⇒ bytes idênticos");

        // E o hash de replay do artefato é estável — é o que a assinatura vai cobrir.
        let ra = ShowReader::new(Cursor::new(a)).unwrap().collect_all().unwrap();
        let rb = ShowReader::new(Cursor::new(b)).unwrap().collect_all().unwrap();
        assert_eq!(pixel_hash(&ra), pixel_hash(&rb));
    }

    #[test]
    fn audio_snapshots_survive_the_cut() {
        use crate::AudioSnapshot;
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = ShowWriter::new(&mut buf, 10).unwrap();
            w.write_frame(&ShowRecord {
                timestamp_ms: 7,
                pixels: vec![PixelColor::rgb(1, 2, 3); 10],
                audio: Some(AudioSnapshot {
                    sample_rate: 48_000,
                    rms: 0.5,
                    beat: true,
                    bpm: 120.0,
                    bass_energy: 0.1,
                    mid_energy: 0.2,
                    high_energy: 0.3,
                }),
            })
            .unwrap();
            finalise_seekable(&mut w).unwrap();
        }
        let (baked, _) = bake_to_vec(&buf.into_inner(), std::slice::from_ref(&(2..5))).unwrap();
        let out = ShowReader::new(Cursor::new(baked)).unwrap().collect_all().unwrap();
        assert!(out[0].audio.is_some(), "o instantâneo de áudio viaja intacto");
        let a = out[0].audio.unwrap();
        assert!(a.beat);
        assert_eq!(a.sample_rate, 48_000, "todos os campos sobrevivem, não só o beat");
        assert_eq!(a.bpm, 120.0);
    }

    #[test]
    fn artifact_bytes_matches_the_real_file() {
        // O número que dimensiona a flash de um traje é derivado, não estimado. Este é o
        // tamanho REAL de robot_sequence.lumyx (6.200 px × 3.925 quadros).
        assert_eq!(artifact_bytes(6200, 3925), 73_040_341);
        // E um traje de 400 px num número de 4 min a 40 fps:
        assert_eq!(artifact_bytes(400, 240 * 40), 11_606_416);
    }

    #[test]
    fn artifact_bytes_agrees_with_what_bake_actually_writes() {
        // O gate que impede a fórmula de virar ficção: ela tem de bater com o arquivo.
        let src = source(120, 11);
        let (baked, frames) = bake_to_vec(&src, std::slice::from_ref(&(3..37))).unwrap();
        assert_eq!(baked.len() as u64, artifact_bytes(34, frames));
    }

    /// **Não vazamento.** O artefato só pode conter os pixels selecionados, na ordem
    /// declarada — nada de fora das faixas pode entrar. Marcadores distintivos dentro e fora
    /// tornam qualquer vazamento uma igualdade que falha, não um "parece certo".
    #[test]
    fn the_artefact_contains_only_the_selected_pixels_in_the_declared_order() {
        const INSIDE_A: PixelColor = PixelColor { r: 11, g: 11, b: 11 };
        const INSIDE_B: PixelColor = PixelColor { r: 22, g: 22, b: 22 };
        // Cor proibida: se ela aparecer no artefato, houve vazamento.
        const OUTSIDE: PixelColor = PixelColor { r: 99, g: 88, b: 77 };

        let ranges = vec![10..13, 40..42];
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = ShowWriter::new(&mut buf, 60).unwrap();
            for f in 0..3u64 {
                let mut px = vec![OUTSIDE; 60]; // tudo proibido por padrão…
                px[10..13].fill(INSIDE_A); // …menos o que as faixas selecionam
                px[40..42].fill(INSIDE_B);
                w.write_frame(&ShowRecord { timestamp_ms: f * 25, pixels: px, audio: None })
                    .unwrap();
            }
            finalise_seekable(&mut w).unwrap();
        }

        let (baked, _) = bake_to_vec(&buf.into_inner(), &ranges).unwrap();
        let out = ShowReader::new(Cursor::new(baked)).unwrap().collect_all().unwrap();

        for (f, rec) in out.iter().enumerate() {
            assert_eq!(rec.pixels.len(), 5, "quadro {f}: 3 + 2 pixels, nem um a mais");
            assert!(
                !rec.pixels.contains(&OUTSIDE),
                "quadro {f}: VAZAMENTO — pixel de fora das faixas entrou no artefato"
            );
            // Ordem declarada: faixa 1 primeiro, faixa 2 depois.
            assert_eq!(
                rec.pixels,
                vec![INSIDE_A, INSIDE_A, INSIDE_A, INSIDE_B, INSIDE_B],
                "quadro {f}: ordem tem de ser a das faixas, concatenadas"
            );
        }
    }

    // ── Controles negativos ───────────────────────────────────────────────────

    /// Faixas sobrepostas são **recusadas**, não duplicadas em silêncio. Duplicação silenciosa
    /// vira dois trechos do traje acendendo juntos — sintoma difícil de atribuir no palco.
    #[test]
    fn negative_control_overlapping_ranges_are_refused_not_silently_duplicated() {
        let src = source(100, 1);
        assert_eq!(
            bake_to_vec(&src, &[0..20, 15..30]).unwrap_err(),
            BakeError::OverlappingRanges { a: 0, b: 1, pixel: 15 }
        );
        // Sobreposição também é pega quando as faixas chegam fora de ordem…
        assert_eq!(
            bake_to_vec(&src, &[50..60, 10..20, 55..58]).unwrap_err(),
            BakeError::OverlappingRanges { a: 0, b: 2, pixel: 55 }
        );
        // …e faixas apenas ADJACENTES continuam legítimas (fim exclusivo, sem toque).
        assert!(bake_to_vec(&src, &[0..10, 10..20]).is_ok(), "adjacente não é sobreposto");
    }


    /// O erro que mata um show: uma faixa que passa do fim. Truncar em silêncio produziria
    /// um traje com menos pixels — e o membro que faltasse só apareceria apagado no palco.
    #[test]
    fn negative_control_out_of_bounds_range_is_refused_not_truncated() {
        let src = source(100, 2);
        let err = bake_to_vec(&src, std::slice::from_ref(&(90..120))).unwrap_err();
        assert_eq!(
            err,
            BakeError::OutOfBounds { index: 0, end: 120, source_pixels: 100 },
            "tem de recusar, nunca truncar"
        );
    }

    #[test]
    fn negative_control_empty_and_degenerate_subsets_are_refused() {
        let src = source(100, 1);
        assert_eq!(bake_to_vec(&src, &[]).unwrap_err(), BakeError::EmptySubset);
        assert_eq!(
            bake_to_vec(&src, std::slice::from_ref(&(50..50))).unwrap_err(),
            BakeError::DegenerateRange { index: 0, start: 50, end: 50 }
        );
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = 60..40;
        assert_eq!(
            bake_to_vec(&src, std::slice::from_ref(&reversed)).unwrap_err(),
            BakeError::DegenerateRange { index: 0, start: 60, end: 40 }
        );
    }

    /// O `bake` não pode ter o show inteiro em memória — é a razão de o módulo existir.
    /// Prova estrutural: assar um show cujo TOTAL é muito maior que qualquer buffer que a
    /// função declara, e verificar que ela conclui e que o resultado está correto.
    /// (O `ShowReader` é alimentado por um `Read` genérico; nada aqui chama `collect_all`.)
    #[test]
    fn bake_streams_and_never_materialises_the_show() {
        // 2.000 quadros × 900 px = 5,4 MB de origem; o recorte segura 1 quadro de cada vez.
        let src = source(900, 2000);
        assert!(src.len() > 5_000_000, "origem tem de ser grande o bastante para valer");
        let (baked, frames) = bake_to_vec(&src, std::slice::from_ref(&(100..500))).unwrap();
        assert_eq!(frames, 2000);
        assert_eq!(baked.len() as u64, artifact_bytes(400, 2000));

        // Amostra o último quadro: o fluxo não se perdeu no caminho.
        let out = ShowReader::new(Cursor::new(baked)).unwrap().collect_all().unwrap();
        assert_eq!(out[1999].timestamp_ms, 1999 * 25);
        assert_eq!(out[1999].pixels[0], PixelColor::rgb(100, 0, (1999 % 256) as u8));
    }
}
