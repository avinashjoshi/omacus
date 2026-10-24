// The installed iPhone app reports a viewport shorter than the screen. Measure
// that strip once, before the page is drawn, and let the shell fill it.
(function () {
  function installed() {
    return window.matchMedia("(display-mode: standalone)").matches || navigator.standalone === true;
  }

  function apply() {
    const root = document.documentElement;
    if (!installed()) {
      root.classList.remove("standalone");
      root.style.removeProperty("--standalone-gap");
      return;
    }
    root.classList.add("standalone");
    let gap = (window.screen && window.screen.height ? window.screen.height : 0) - window.innerHeight;
    // A desktop window is shorter than the monitor by far more than a phone
    // status bar. Leave that alone.
    if (!(gap > 0 && gap < 120)) gap = 0;
    root.style.setProperty("--standalone-gap", gap + "px");
  }

  apply();
  addEventListener("resize", apply);
  addEventListener("orientationchange", apply);
  if (window.visualViewport) visualViewport.addEventListener("resize", apply);
})();
