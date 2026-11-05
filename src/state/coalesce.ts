/**
 * Runs `task` one at a time: a request while it runs makes it run once more
 * afterwards, so the last change is always read and reads never pile up.
 */
export function coalesced(task: () => Promise<void>): () => void {
  let running = false;
  let again = false;
  const run = async () => {
    if (running) {
      again = true;
      return;
    }
    running = true;
    try {
      do {
        again = false;
        await task();
      } while (again);
    } finally {
      running = false;
    }
  };
  return () => void run();
}
