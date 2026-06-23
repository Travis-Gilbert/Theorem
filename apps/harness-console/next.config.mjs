/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // This app is its own Vercel project; scope file tracing to it so the parent
  // monorepo's lockfiles don't get inferred as the workspace root.
  outputFileTracingRoot: import.meta.dirname,
};

export default nextConfig;
