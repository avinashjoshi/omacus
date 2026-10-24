// Home Screen cold start: env(safe-area-inset-*) can be 0 until the viewport
// has been exercised, and a percentage height drops viewport-fit=cover.
// Measure the insets with probe elements, then measure them again. The shell
// height stays in CSS (100vh). Do not size it from the window.
(function () {
  function installed() {
    return window.navigator.standalone === true ||
      window.matchMedia("(display-mode: standalone)").matches;
  }

  function measureEnv(prop) {
    var el = document.createElement("div");
    el.style.cssText = "position:fixed;top:0;left:0;width:0;height:env(" +
      prop + ", 0px);visibility:hidden;pointer-events:none;";
    document.body.appendChild(el);
    var val = el.offsetHeight;
    el.remove();
    return val;
  }

  function update() {
    if (!document.body) return;
    var root = document.documentElement;
    if (installed()) root.classList.add("standalone");
    else root.classList.remove("standalone");
    document.body.style.setProperty("--probed-top", measureEnv("safe-area-inset-top") + "px");
    document.body.style.setProperty("--probed-bottom", measureEnv("safe-area-inset-bottom") + "px");
    document.body.style.setProperty("--probed-left", measureEnv("safe-area-inset-left") + "px");
    document.body.style.setProperty("--probed-right", measureEnv("safe-area-inset-right") + "px");
  }

  function boot() {
    update();
    setTimeout(update, 100);
    setTimeout(update, 500);
    setTimeout(update, 1000);
    if (window.navigator.standalone) {
      var meta = document.querySelector('meta[name="viewport"]');
      var original = meta && meta.getAttribute("content");
      if (original && original.indexOf("viewport-fit=cover") !== -1) {
        meta.setAttribute("content", original.replace("viewport-fit=cover", "viewport-fit=auto"));
        requestAnimationFrame(function () {
          meta.setAttribute("content", original);
          requestAnimationFrame(update);
        });
      }
    }
    addEventListener("resize", update);
    addEventListener("orientationchange", function () {
      setTimeout(update, 300);
    });
    if (window.visualViewport) visualViewport.addEventListener("resize", update);
  }

  if (document.body) boot();
  else addEventListener("DOMContentLoaded", boot, { once: true });
})();
