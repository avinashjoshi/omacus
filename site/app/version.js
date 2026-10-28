// The release this shell was built from. prepare-release.sh bumps it with
// Cargo.toml, the sidebar, and the service worker. The running copy reports
// this string, which is how an open page can tell it is older than the network.
export const APP_VERSION = "1.0.9";
