// How long a joining phone can go without asking before it is not waiting.
// This is its own module. The worker entry cannot export a plain number.
export const HEARTBEAT_MS = 30 * 1000;
