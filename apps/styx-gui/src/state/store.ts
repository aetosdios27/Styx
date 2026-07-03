import { createContext, Dispatch, useContext } from "react";
import { AppAction, AppState } from "./reducer";

export const AppStateContext = createContext<AppState | null>(null);
export const AppDispatchContext = createContext<Dispatch<AppAction> | null>(null);

export function useAppState(): AppState {
  const state = useContext(AppStateContext);
  if (!state) {
    throw new Error("AppStateContext is missing");
  }
  return state;
}

export function useAppDispatch(): Dispatch<AppAction> {
  const dispatch = useContext(AppDispatchContext);
  if (!dispatch) {
    throw new Error("AppDispatchContext is missing");
  }
  return dispatch;
}
