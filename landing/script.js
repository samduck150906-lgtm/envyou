/* envyou landing — light progressive enhancement.
   No framework, no build step. */
(function () {
  "use strict";

  // Footer year.
  var y = document.getElementById("year");
  if (y) y.textContent = String(new Date().getFullYear());

  // Highlight the visitor's likely platform in the download section.
  var ua = navigator.userAgent || "";
  var platform = "macos";
  if (/Windows/i.test(ua)) platform = "windows";
  else if (/Linux/i.test(ua) && !/Android/i.test(ua)) platform = "linux";
  else if (/Mac/i.test(ua)) platform = "macos";

  var preferred = document.querySelector('.dl-card[data-platform="' + platform + '"]');
  if (preferred) {
    preferred.style.outline = "2px dotted #000080";
    preferred.setAttribute("title", "추천: 현재 사용 중인 OS");
  }

  // Placeholder download links: builds aren't published yet, so intercept
  // clicks and surface a friendly notice instead of navigating to "#".
  var note = document.getElementById("dl-note");
  document.querySelectorAll(".dl-card").forEach(function (card) {
    card.addEventListener("click", function (e) {
      var href = card.getAttribute("href");
      if (!href || href === "#") {
        e.preventDefault();
        if (note) {
          var os = card.getAttribute("data-platform");
          note.textContent =
            "⏳ " + os + " 빌드는 곧 공개됩니다. GitHub Releases를 지켜봐 주세요!";
        }
      }
    });
  });
})();
