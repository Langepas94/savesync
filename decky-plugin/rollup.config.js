import commonjs from "@rollup/plugin-commonjs";
import json from "@rollup/plugin-json";
import { nodeResolve } from "@rollup/plugin-node-resolve";
import replace from "@rollup/plugin-replace";
import typescript from "@rollup/plugin-typescript";

export default {
  input: "index.tsx",
  plugins: [
    commonjs(),
    nodeResolve(),
    json(),
    typescript(),
    replace({
      preventAssignment: false,
      "process.env.NODE_ENV": JSON.stringify("production"),
    }),
  ],
  context: "window",
  external: ["react", "react-dom", "@decky/api", "@decky/ui"],
  output: {
    file: "dist/index.js",
    name: "SaveSync",
    globals: {
      react: "SP_REACT",
      "react-dom": "SP_REACTDOM",
      "@decky/api": "DECKY_API",
      "@decky/ui": "DECKY_UI",
    },
    format: "iife",
    exports: "default",
  },
};
