import { defineConfig } from "tsup";

export default defineConfig({
  entry: { index: "src/index.ts" },
  format: ["esm", "cjs"],
  // Declarations are emitted by `tsc` (see the `build` script), not tsup.
  // tsup's `dts` uses rollup-plugin-dts, which as of 6.4.x still caps its
  // TypeScript peer at ^6.0 and crashes under TS 7 (`useCaseSensitiveFile-
  // Names` of undefined). `tsc --emitDeclarationOnly` uses the real TS 7
  // compiler, so declarations track whatever TypeScript we build with.
  dts: false,
  sourcemap: true,
  clean: true,
  target: "node18",
});
