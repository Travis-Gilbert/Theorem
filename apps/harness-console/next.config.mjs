import path from "node:path";

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // This app is its own Vercel project; scope file tracing to it so the parent
  // monorepo's lockfiles don't get inferred as the workspace root.
  outputFileTracingRoot: path.join(import.meta.dirname),
  // The console is its own Vercel project. ESLint is advisory here; do not let a
  // lint nit block a production build of the surface.
  eslint: { ignoreDuringBuilds: true },
  // cosmos.gl and d3 ship ESM that Next can bundle; keep the transpile list
  // explicit so the GPU/graph lane stays isolated and tree-shakeable.
  transpilePackages: ["d3"],
};

export default nextConfig;
