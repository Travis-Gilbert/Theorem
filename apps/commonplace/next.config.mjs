/** @type {import('next').NextConfig} */
const nextConfig = {
  output: "export",
  outputFileTracingRoot: import.meta.dirname,
  images: {
    unoptimized: true
  },
  trailingSlash: true,
  turbopack: {
    root: import.meta.dirname
  }
};

export default nextConfig;
