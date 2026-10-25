// IPC with one rule: at most one call of a kind in flight, the latest
// arguments win, errors are reported but never clear the last result.
// window.__TAURI__ is read only when invoke runs, never at import, so the
// module loads under node where the tests hand coalesced a fake invoke.
export const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);

export function coalesced(name, onResult, onError, invokeFn = invoke) {
  let busy = false;
  let again = null;
  async function run(args) {
    if (busy) {
      again = args;
      return;
    }
    busy = true;
    try {
      onResult(await invokeFn(name, args));
    } catch (e) {
      onError(String(e && e.message ? e.message : e));
    } finally {
      busy = false;
      if (again) {
        const a = again;
        again = null;
        run(a);
      }
    }
  }
  return run;
}
