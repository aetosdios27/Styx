import { useEffect, useReducer } from "react";
import { getSnapshot, subscribeToEvents } from "./api/client";
import { Shell } from "./components/Shell";
import { AppDispatchContext, AppStateContext } from "./state/store";
import { initialState, reducer } from "./state/reducer";

export default function App() {
  const [state, dispatch] = useReducer(reducer, undefined, initialState);

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | null = null;

    getSnapshot()
      .then((snapshot) => {
        if (!disposed) {
          dispatch({ type: "snapshot_received", snapshot });
        }
      })
      .catch((error: unknown) => {
        if (!disposed) {
          dispatch({ type: "command_failed", error: String(error) });
        }
      });

    subscribeToEvents((event) => dispatch({ type: "event_received", event }))
      .then((unlisten) => {
        cleanup = unlisten;
      })
      .catch(() => {
        // Browser-only test/dev runs do not have the Tauri event bridge.
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  return (
    <AppStateContext.Provider value={state}>
      <AppDispatchContext.Provider value={dispatch}>
        <Shell />
      </AppDispatchContext.Provider>
    </AppStateContext.Provider>
  );
}
