//! ADR-0026 §9-bis — `/api/metrics` atravessa o console, e atravessa-o **inalterado**.
//!
//! Não há mocks: sobe o `led_hal::serve_metrics` **real** e compara o que o console devolve
//! com o que o exporter produziu. Um exporter de mentira concordaria sempre comigo — e a
//! única coisa que este teste existe para apanhar é precisamente uma divergência entre os
//! dois.

use led_console_bin::metrics::{buscar, ErroMetricas};
use led_hal::{prometheus_text, serve_metrics, MetricsEmitter};
use std::net::SocketAddr;
use std::sync::Arc;

fn emissor_com_dados() -> Arc<MetricsEmitter> {
    let e = Arc::new(MetricsEmitter::new("proxy-test"));
    e.record_frame(1_000);
    e.record_frame(2_000);
    e.record_frame(3_500);
    e.record_drop();
    e.record_beat();
    e
}

/// **O corpo chega byte a byte igual ao do exporter.**
///
/// Esta é a asserção que prova as quatro proibições do §9-bis de uma vez: não recalcula, não
/// agrega, não converte e não reescreve o formato. Qualquer uma delas mudaria pelo menos um
/// byte, e a comparação é contra o `prometheus_text` **do próprio exporter**, não contra uma
/// string que eu tenha escrito à mão.
#[test]
fn o_corpo_atravessa_o_console_inalterado() {
    let e = emissor_com_dados();
    let srv = serve_metrics(vec![Arc::clone(&e)], "127.0.0.1:0".parse().unwrap()).expect("bind");

    let esperado = prometheus_text(std::slice::from_ref(&e));
    let obtido = buscar(srv.addr).expect("o console tem de conseguir falar com o exporter");

    assert_eq!(
        obtido.corpo, esperado,
        "o corpo foi alterado entre o exporter e o console — o proxy tem de repassar verbatim"
    );
    assert!(
        obtido.corpo.contains("lumyx_frames_total"),
        "sanidade: sem isto, comparar duas strings vazias passaria sem provar nada\n{}",
        obtido.corpo
    );
}

/// **O `Content-Type` do Prometheus sobrevive à passagem.**
///
/// `version=0.0.4` é o que identifica o formato de exposição. Um proxy que o reescrevesse
/// faria um scraper legítimo interpretar o corpo com outro parser.
#[test]
fn o_content_type_do_prometheus_nao_e_reescrito() {
    let e = emissor_com_dados();
    let srv = serve_metrics(vec![e], "127.0.0.1:0".parse().unwrap()).expect("bind");

    let obtido = buscar(srv.addr).expect("buscar");
    assert!(
        obtido.content_type.contains("version=0.0.4"),
        "o formato de exposicao foi reescrito: {:?}",
        obtido.content_type
    );
    assert!(obtido.content_type.starts_with("text/plain"), "{:?}", obtido.content_type);
}

/// **Exporter em baixo é um estado, não uma métrica fabricada.**
///
/// É a forma mais fácil de este proxy mentir: devolver corpo vazio e o browser desenhar
/// zeros — indistinguível de um rig parado. `Instantaneo::nunca_houve` e o `Veredito` do
/// `hwcheck` já tomaram esta decisão; aqui ela é a mesma.
#[test]
fn exporter_em_baixo_nunca_vira_metricas_vazias() {
    // Porta fechada: subimos e **paramos** o exporter.
    //
    // A 1.ª versão deste teste usava `drop(srv)` e falhou — e a falha era minha, não do
    // proxy: o `MetricsServer` documenta que *"dropping it does NOT stop the server"*. O
    // servidor continuava a atender, o proxy devolvia um 200 legítimo, e o meu `panic!`
    // acusava-o de fabricar. Ler o contrato antes de o usar teria poupado o desvio.
    let addr: SocketAddr = {
        let srv = serve_metrics(vec![], "127.0.0.1:0".parse().unwrap()).expect("bind");
        let a = srv.addr;
        srv.stop();
        a
    };

    match buscar(addr) {
        Err(ErroMetricas::ExporterOffline(_)) => {}
        Err(outro) => panic!("erro errado para exporter em baixo: {outro:?}"),
        Ok(m) => panic!(
            "o proxy FABRICOU uma resposta com o exporter em baixo: {:?}",
            m.corpo
        ),
    }
}

/// O estado HTTP com que o console responde ao browser **culpa o componente certo**.
///
/// 503/502 e nunca 500: a falha é a montante. Dizer 500 mandaria o operador procurar o
/// defeito no console — a mesma inversão de culpa que o `PedidoDemasiadoGrande` corrigiu.
#[test]
fn a_falha_do_exporter_nao_e_reportada_como_falha_do_console() {
    assert_eq!(ErroMetricas::ExporterOffline("x".into()).http_status(), 503);
    assert_eq!(ErroMetricas::ExporterRecusou(404).http_status(), 502);
    assert_eq!(ErroMetricas::RespostaInvalida("x".into()).http_status(), 502);
}
