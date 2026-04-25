import type { GunParameters, GunSolution } from './worker-protocol';

export interface AppState {
  // The currently selected parameters.
  parameters: GunParameters;

  // Whether parameters have changed since the last solve request was sent.
  pendingParameters: boolean;

  // The most recently computed solution (if any).
  solution: GunSolution | null;

  // Whether a solution is currently being computed.
  solving: boolean;
}
