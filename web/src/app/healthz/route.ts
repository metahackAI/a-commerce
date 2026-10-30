// Liveness for the container and Kubernetes probes (infra/README.md).
export const dynamic = 'force-dynamic';

export function GET() {
  return Response.json({status: 'ok', service: 'web'});
}
