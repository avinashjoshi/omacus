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
