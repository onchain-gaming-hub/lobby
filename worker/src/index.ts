// Worker entry point. ⚠️ Export ONLY `default` here: workerd treats every named export of the main module as
// an entrypoint and refuses to load a non-function one (P7 gate R4-1). Logic + constants live in ./worker.
import { CRON_JOBS, type Env, log, run } from './worker';

export default {
  async scheduled(controller: ScheduledController, env: Env): Promise<void> {
    const job = CRON_JOBS[controller.cron];
    if (!job) return log({ level: 'error', msg: 'unknown cron', cron: controller.cron });
    await run(job, env); // awaited: the cron is reported finished only when the work is (P7 gate N5)
  },
  async fetch(): Promise<Response> {
    return new Response('lobby-worker: cron only', { status: 404 });
  },
} satisfies ExportedHandler<Env>;
