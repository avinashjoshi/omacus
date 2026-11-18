# Compact vault navigation

These are captures of the running apps, with example pages. The web fixture supplies a fictional Personal vault and a local page through the existing sync test hook; it does not connect to a real sync account. The native captures use a separate local sample library.

`before/` records the previous navigation. `after/` records the compact header, proportional page names, full-width phone browser, and quieter footer. The phone captures use a touch-enabled 390 × 844 viewport. The desktop web captures are 1120 × 760. Native screenshots include the display's scale factor.

To capture the web views after building both WebAssembly packages:

```sh
OMACUS_NAV_SHOTS=docs/design/vault-navigation/after npx playwright test vault-navigation --grep screenshots --project chromium
```

The screenshot test also accepts `OMACUS_NAV_BEFORE=1` to load the committed HTML, CSS, and app script from HEAD, before committing the implementation.

The interaction tests cover vault selection, scoped search (including collapsed folders), creation destinations, scoped permanent deletion, and phone touch targets. Multiple synced vaults remain future work; the picker currently presents the existing synced vault and local pages.

The latest native capture uses a fresh example library and bundled Inter. The desktop vault menu now shares the web menu’s two-line labels and selected checkmark. A session regression test covers local and locked page membership while background sync owns the live session.
