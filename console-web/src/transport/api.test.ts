// ─────────────────────────────────────────────────────────────────────────────
// Testes da interpretação de erros. **Função pura, sem duplos de rede.**
//
// Os corpos abaixo são os que o console emite de facto (`http.rs::Saida::erro`), e
// os códigos são reais: `console.daemon_offline` vem do `ipc.rs`, `no_show_loaded`
// do `Rejected::code` do runtime. Nenhum foi inventado para o teste.
// ─────────────────────────────────────────────────────────────────────────────

import { describe, expect, it } from "vitest";
import { interpretarErro } from "./api";

describe("interpretarErro", () => {
  it("deixa o codigo do backend passar VERBATIM", () => {
    // É a regra do ADR-0026 §6: o código do daemon atravessa intacto. Reescrevê-lo aqui
    // apagaria a razão da falha, e o operador procuraria o defeito no sítio errado.
    const corpo = JSON.stringify({
      ok: false,
      error: { code: "no_show_loaded", detail: "NoShowLoaded" },
    });
    expect(interpretarErro(409, corpo)).toEqual({
      code: "no_show_loaded",
      detail: "NoShowLoaded",
    });
  });

  it("preserva o codigo de OFFLINE, que e um estado e nao um erro nosso", () => {
    const corpo = JSON.stringify({
      ok: false,
      error: { code: "console.daemon_offline", detail: "/tmp/x.sock: No such file" },
    });
    const r = interpretarErro(503, corpo);
    expect(r.code).toBe("console.daemon_offline");
    expect(r.detail).toContain("/tmp/x.sock");
  });

  it("corpo ilegivel NAO vira um codigo do backend", () => {
    // O prefixo `console-web.` existe para que ninguém confunda uma falha do cliente com
    // uma recusa do daemon. Se isto devolvesse um código sem prefixo, um erro de parsing
    // no browser pareceria um veredito do backend.
    for (const lixo of ["", "isto nao e json", "<html>502</html>", "null"]) {
      const r = interpretarErro(502, lixo);
      expect(r.code).toBe("console-web.bad_response");
      expect(r.detail).toContain("502");
    }
  });

  it("um JSON valido SEM a forma de erro tambem cai no codigo do cliente", () => {
    // JSON analisável mas sem `error.code` não é uma recusa do daemon — é uma resposta
    // que não se percebeu, e dizer isso é mais honesto do que inventar um código.
    for (const corpo of ['{"ok":false}', '{"error":{}}', '{"error":"texto"}', "[1,2,3]"]) {
      expect(interpretarErro(500, corpo).code).toBe("console-web.bad_response");
    }
  });

  it("o estado HTTP entra no detalhe quando nao ha corpo utilizavel", () => {
    // Sem isto, uma falha sem corpo daria uma mensagem vazia e o operador ficaria sem
    // nada para procurar.
    expect(interpretarErro(413, "").detail).toBe("HTTP 413");
  });
});
