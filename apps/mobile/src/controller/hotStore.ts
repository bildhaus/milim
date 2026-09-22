import {useSyncExternalStore} from 'react';
import type {TimelineReplica} from '../control/replica';

// State that changes many times per second (streamed timeline events, composer
// keystrokes). It lives outside React state so an update re-renders only the
// components that select it, not everything below the controller.
export type HotState = {
  timeline: TimelineReplica | null;
  draft: string;
};

export type HotStore = {
  get: () => HotState;
  set: (patch: Partial<HotState>) => void;
  subscribe: (listener: () => void) => () => void;
};

export function createHotStore(): HotStore {
  let state: HotState = {timeline: null, draft: ''};
  const listeners = new Set<() => void>();
  return {
    get: () => state,
    set(patch) {
      const next = {...state, ...patch};
      if (next.timeline === state.timeline && next.draft === state.draft) return;
      state = next;
      for (const listener of [...listeners]) listener();
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

// The selector must return a referentially stable value for unchanged state
// (a field, a primitive, or a derived primitive), as useSyncExternalStore requires.
export function useHotState<T>(store: HotStore, select: (state: HotState) => T): T {
  return useSyncExternalStore(store.subscribe, () => select(store.get()));
}
