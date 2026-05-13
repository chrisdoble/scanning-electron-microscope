import type { GunParameters, GunSolution } from './worker-protocol';
import type { Trajectory } from './trajectories';

export interface AppState {
  // The currently selected parameters.
  parameters: GunParameters;

  // Whether parameters have changed since the last solve request was sent.
  pendingParameters: boolean;

  // The most recently computed solution (if any).
  solution: GunSolution | null;

  // Electron trajectories computed from the most recent solution.
  trajectories: Trajectory[];

  // Whether a solution is currently being computed.
  solving: boolean;
}
