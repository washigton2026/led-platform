//! ADR-0026 §11–12 — os limites da ponte, **derivados**, nunca reescritos.

use std::net::SocketAddr;
use std::time::Duration;

/// Corpo máximo aceite do browser. **É o mesmo `MAX_LINE` do GS3**, e pela mesma razão: sem
/// um teto, um cliente que nunca feche o pedido faz o processo crescer sem limite.
pub const MAX_BODY: usize = led_daemon_bin::server::MAX_LINE;

/// Profundidade máxima de JSON. **É o mesmo `MAX_DEPTH` do GS3**: `[[[[[…` recursivo estoura
/// a pilha, e um cliente derrubaria o processo com uma linha de texto.
pub const MAX_JSON_DEPTH: usize = led_daemon_bin::json::MAX_DEPTH;

/// Quanto tempo o console espera **além** do que o daemon espera pelo laço.
///
/// Existe como constante nomeada para que a soma seja legível e para que ninguém a
/// transforme num número solto.
pub const MARGEM_HTTP: Duration = Duration::from_secs(2);

/// Teto de ligações SSE simultâneas. Separadores esquecidos não podem esgotar threads.
pub const MAX_SSE_CONNS: usize = 8;

/// O timeout HTTP do console — **derivado** do `REPLY_TIMEOUT` do daemon (ADR-0026 §12).
///
/// # Porque é derivado, e não escrito
///
/// O daemon desiste de esperar pelo laço em `REPLY_TIMEOUT`. Se o console desistisse **antes**
/// ou **ao mesmo tempo**, o browser receberia "falhou" enquanto o daemon ainda aplicaria o
/// comando — e o operador veria o show mudar depois de a UI ter dito que não mudou. Escrever
/// um segundo `Duration::from_secs(5)` aqui empataria os dois no dia em que um deles mudasse.
pub fn http_timeout() -> Duration {
    led_daemon_bin::server::REPLY_TIMEOUT + MARGEM_HTTP
}

/// **ADR-0026 §10 — loopback-only enquanto o `ClientRegistry` do ADR-0014 estiver vazio.**
///
/// Recusar por construção é mais forte que verificar em runtime: é o mesmo desenho que o
/// `led-readmodel` já usa (`serve_readmodel` recusa bind não-loopback) e que o GS3 usou ao
/// não ter TCP de todo.
pub fn bind_permitido(addr: SocketAddr) -> Result<(), String> {
    if addr.ip().is_loopback() {
        return Ok(());
    }
    Err(format!(
        "bind em {} recusado: o console e loopback-only ate o ADR-0014 fornecer \
         autenticacao (ClientRegistry esta vazio). Use 127.0.0.1",
        addr.ip()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **O teste que impede a janela perigosa.** Se alguém escrever um número à mão aqui e o
    /// `REPLY_TIMEOUT` mudar, este teste é o que fica vermelho.
    #[test]
    fn o_timeout_http_e_estritamente_maior_que_o_do_daemon() {
        let daemon = led_daemon_bin::server::REPLY_TIMEOUT;
        assert!(
            http_timeout() > daemon,
            "HTTP {:?} tem de ser MAIOR que o REPLY_TIMEOUT {daemon:?}; caso contrario o \
             browser desiste enquanto o daemon ainda aplica",
            http_timeout()
        );
        assert_eq!(http_timeout(), daemon + MARGEM_HTTP, "e tem de ser DERIVADO, nao escrito");
        assert!(MARGEM_HTTP > Duration::ZERO, "margem zero empata os dois");
    }

    /// Os limites do GS3 são os mesmos objetos, não cópias com o mesmo valor.
    #[test]
    fn os_limites_sao_os_do_gs3_e_nao_copias() {
        assert_eq!(MAX_BODY, led_daemon_bin::server::MAX_LINE);
        assert_eq!(MAX_JSON_DEPTH, led_daemon_bin::json::MAX_DEPTH);
        assert_eq!(MAX_BODY, 64 * 1024, "e o valor continua a ser o do GS3");
        assert_eq!(MAX_JSON_DEPTH, 16);
    }

    #[test]
    fn loopback_e_aceite_e_tudo_o_resto_e_recusado() {
        for ok in ["127.0.0.1:8080", "[::1]:8080"] {
            assert!(bind_permitido(ok.parse().unwrap()).is_ok(), "{ok}");
        }
        for mau in ["0.0.0.0:8080", "192.168.1.10:8080", "10.0.0.1:80", "[::]:8080"] {
            let e = bind_permitido(mau.parse().unwrap()).unwrap_err();
            assert!(e.contains("ADR-0014"), "{mau}: a recusa tem de dizer porque: {e}");
        }
    }
}
