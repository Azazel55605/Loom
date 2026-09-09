type BackHandler = () => void;

type BackHandlerEntry = {
  handler: BackHandler;
};

const handlers: BackHandlerEntry[] = [];

/**
 * Last-opened, first-closed overlay handlers.
 *
 * Shared overlays register here on every platform. Only Mobile consumes the
 * stack from a hardware-back listener, so Web and Desktop pay no behavioral
 * cost for participating.
 */
export const BackHandlerStack = {
  register(handler: BackHandler): () => void {
    const entry = { handler };
    handlers.push(entry);

    return () => {
      const index = handlers.lastIndexOf(entry);
      if (index !== -1) handlers.splice(index, 1);
    };
  },

  handleBackPress(): boolean {
    const entry = handlers.pop();
    if (entry === undefined) return false;
    entry.handler();
    return true;
  },
};
