// Trusted Types for the web app. The page builds its UI with DOM nodes.
// A policy named omacus exists so a future string can be marked on purpose.
// There is no default policy: assigning HTML to the page throws.

(function () {
  let policy = null;
  if (window.trustedTypes && typeof trustedTypes.createPolicy === "function") {
    try {
      policy = trustedTypes.createPolicy("omacus", {
        createHTML() {
          throw new TypeError("This page does not build UI from HTML strings.");
        },
        createScript() {
          throw new TypeError("This page does not build scripts from strings.");
        },
        createScriptURL(input) {
          const url = String(input);
          const same = url.startsWith(location.origin + "/") || url.startsWith("./") || url.startsWith("pkg/") || url.startsWith("/");
          if (!same) throw new TypeError("That address was refused.");
          return url;
        },
      });
    } catch {
      // A second copy of this file in the same document leaves the first policy.
    }
  }
  // Workers and the service worker take a script address. The page asks for
  // one here. There is no default policy, so an HTML string still cannot land.
  globalThis.omacusScriptURL = function (input) {
    const url = String(input);
    return policy ? policy.createScriptURL(url) : url;
  };
})();
