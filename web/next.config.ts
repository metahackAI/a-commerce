import type {NextConfig} from 'next';

// `standalone` builds a self-contained server (infra/docker/web.Dockerfile runs `node server.js`).
const config: NextConfig = {
  output: 'standalone',
  reactStrictMode: true,
  poweredByHeader: false,
};

export default config;
