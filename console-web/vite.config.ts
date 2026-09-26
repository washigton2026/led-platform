import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/**
 * O `proxy` não é conveniência: é o que mantém a **mesma origem**.
 *
 * Sem ele, o browser pediria de `localhost:5173` para `127.0.0.1:7878` e isso seria uma
 * requisição cross-origin — que hoje falharia, porque o console **não emite cabeçalhos CORS**
 * (verificado: zero `Access-Control-*` em `http.rs`). A saída fácil seria acrescentar CORS
 * permissivo ao console; o ADR-0028 D7 proíbe-o, e com razão — um console loopback-only que
 * aceita qualquer origem deixa de ser loopback-only na prática.
 *
 * Com o proxy, o browser vê uma só origem e o console continua sem CORS.
 *
 * `LUMYX_CONSOLE` permite apontar para outra porta sem editar este ficheiro. Não há omissão
 * inventada: o valor abaixo é o exemplo que o `--help` do `led-console` usa.
 */
// Este ficheiro corre em Node (é config de build, não código de browser). Declarar o
// símbolo aqui evita acrescentar `@types/node` inteiro por causa de uma variável — e
// mantém o `vite.config.ts` dentro do typecheck em vez de o excluir dele.
declare const process: { readonly env: Readonly<Record<string, string | undefined>> };

const CONSOLE = process.env.LUMYX_CONSOLE ?? "http://127.0.0.1:7878";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": { target: CONSOLE, changeOrigin: false },
    },
  },
});
