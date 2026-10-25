// A guess for this phone or computer. The person can change it.
// omacus Test and the desktop app put their own name in the browser string.
// WebKit on a computer that also says Chrome is the test app.

export function guessDeviceName(ua) {
  const text = String(ua || "");
  const phone = /iPhone/.test(text) ? "iPhone" : /iPad/.test(text) ? "iPad" : /Android/.test(text) ? "Android" : "";
  let computer = "computer";
  if (/Mac OS X|Macintosh/.test(text)) computer = "Mac";
  else if (/Windows/.test(text)) computer = "Windows";
  else if (/CrOS/.test(text)) computer = "Chromebook";
  else if (/Linux|X11/.test(text)) computer = "Linux";
  const place = phone || computer;
  if (/\bomacus-test\b/.test(text)) return `omacus Test on ${place}`;
  if (/\bomacus-desktop\b/.test(text)) return `Omacus on ${place}`;
  const webkit = /AppleWebKit\/(\d+)/.exec(text);
  const webkitMajor = webkit ? Number(webkit[1]) : 0;
  if (!phone && /Chrome\//.test(text) && webkitMajor >= 600) return `omacus Test on ${place}`;
  let browser = "Browser";
  if (/Edg\//.test(text)) browser = "Edge";
  else if (/Chrome\//.test(text)) browser = "Chrome";
  else if (/Safari\//.test(text)) browser = "Safari";
  else if (/Firefox\//.test(text)) browser = "Firefox";
  if (phone && (browser === "Safari" || browser === "Browser")) return phone;
  return phone ? `${browser} on ${phone}` : `${browser} on ${computer}`;
}
