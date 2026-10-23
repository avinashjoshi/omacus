//! The monthly example refresh cannot push to main. Main is protected.

use std::path::Path;

#[test]
fn the_monthly_refresh_opens_a_pull_request() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join(".github/workflows/freshen.yml")).unwrap();
    assert!(
        !text.contains("HEAD:main"),
        "freshen must not push straight to main"
    );
    assert!(
        text.contains("gh pr create"),
        "freshen should open a pull request"
    );
    assert!(
        text.contains("gh pr merge --auto --squash"),
        "freshen should enable auto-merge"
    );
}

#[test]
fn a_push_still_publishes_when_prepare_is_skipped() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    // prepare and wait are skipped on push. Later jobs need always(), or GitHub
    // skips them even after check says the version is new.
    for job in [
        "ci:",
        "linux:",
        "windows:",
        "macos:",
        "publish:",
        "install-test:",
    ] {
        let at = text.find(&format!("\n  {job}")).expect(job);
        let window = &text[at..text.len().min(at + 500)];
        assert!(
            window.contains("always()"),
            "{job} must use always() so a skipped wait does not skip publishing"
        );
    }
}
