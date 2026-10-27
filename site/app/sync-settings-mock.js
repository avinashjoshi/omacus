// The Sync tab, using the same cards and buttons as Settings.
// Demo names and words only. This does not open a mailbox.

const WORDS = [
  "harbor", "ferry", "oats", "bread", "cake", "pantry",
  "kept", "meadow", "lantern", "river", "cedar", "maple",
];

const SCREENS = [
  "intro", "fresh", "words", "check", "join-words", "join-wait",
  "approve", "approve-timeout", "devices", "remove", "paused",
  "list-changed", "keyring", "off",
];

export function mockScreens() {
  return SCREENS;
}

export const DEMO_TRASH = [{ id: "kept", text: "# Kept\nold loaf\n", handle: "" }];

export function demoLibrary() {
  return {
    tree: [{
      type: "folder",
      id: "pantry",
      name: "Pantry",
      collapsed: false,
      children: [
        { type: "page", id: "oats" },
        { type: "page", id: "ferry" },
        { type: "page", id: "bread" },
        { type: "page", id: "cake" },
      ],
    }],
    texts: {
      oats: "# Oats\nbowl 2\n",
      ferry: "# Ferry\nbowl 2\n",
      bread: "# Bread\nbowl 2\n",
      cake: "# Cake\nbowl 2\n",
    },
    handles: { oats: "oats", ferry: "pier", bread: "bread", cake: "cake" },
    active: "oats",
    seeded: true,
  };
}

function el(tag, attrs, kids) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs || {})) {
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key === "checked") node.checked = value !== "false";
    else node.setAttribute(key, value);
  }
  for (const kid of kids || []) node.append(kid);
  return node;
}

function help(text) {
  return el("p", { class: "setting-help", text });
}

function group(title, kids) {
  return el("section", { class: "settings-group" }, [el("h3", { text: title }), ...kids]);
}

function btn(label, kind) {
  const face = kind === "primary" ? "btn small primary" : kind === "danger" ? "btn small danger" : "btn small";
  return el("button", { type: "button", class: face, text: label });
}

function actions(buttons) {
  return el("div", { class: "setting-actions" }, buttons);
}

function field(label, value, mono) {
  const input = el("input", {
    class: mono ? "sync-mock-field mono" : "sync-mock-field",
    value: value || "",
    readonly: "",
    "aria-label": label,
  });
  return el("div", {}, [el("div", { class: "setting-label", text: label }), input]);
}

function words() {
  return el("div", { class: "sync-mock-words" }, WORDS.map((word, i) => el("div", {
    class: "sync-mock-word",
    text: `${i + 1}. ${word}`,
  })));
}

function choices(title, names) {
  return el("div", {}, [
    el("div", { class: "setting-label", text: title }),
    el("div", { class: "sync-mock-choices" }, names.map((name) => el("button", {
      type: "button",
      class: "sync-mock-choice",
      text: name,
    }))),
  ]);
}

function code(text) {
  return el("p", { class: "sync-mock-code", text });
}

function device(name, kind, canRemove) {
  const row = el("div", { class: "setting-row" }, [
    el("div", {}, [
      el("div", { class: "setting-label", text: name }),
      help(kind),
    ]),
  ]);
  if (canRemove) row.append(btn("Remove", "quiet"));
  return row;
}

function screen(name) {
  if (name === "fresh") {
    return group("Start fresh on this computer", [
      help("Step 1 of 4. Your pages get encrypted here before they leave. The server stores them but can't read them."),
      field("Name this computer", "Harbor desk"),
      help("Shown on your other devices so you can tell them apart."),
      field("Invite code", "", true),
      help("Code you were given. Not the code from another device."),
      actions([btn("Continue", "primary"), btn("Back", "quiet")]),
    ]);
  }
  if (name === "words") {
    return group("Save your recovery words", [
      help("Step 2 of 4. These 12 words are the only way back in if you lose every device. Omacus never keeps a copy, and you won't see them again."),
      words(),
      el("label", { class: "setting-row" }, [
        el("div", {}, [el("div", { class: "setting-label", text: "I wrote these down or printed them somewhere safe." })]),
        el("input", { class: "switch", type: "checkbox", checked: "", "aria-label": "I wrote these down" }),
      ]),
      help("A clipboard history may keep these words. Printing them, or writing them down, is safest."),
      actions([btn("I saved them", "primary"), btn("Print", "quiet"), btn("Copy", "quiet"), btn("Back", "quiet")]),
    ]);
  }
  if (name === "check") {
    return group("Do these match your paper?", [
      help("Step 3 of 4. Pick word 3, word 7, and word 11."),
      choices("Word 3", ["oats", "bread", "ferry", "cake", "harbor", "pantry"]),
      choices("Word 7", ["kept", "meadow", "maple", "cedar", "river", "lantern"]),
      choices("Word 11", ["cedar", "maple", "oats", "bread", "ferry", "cake"]),
      actions([btn("These match", "primary"), btn("Back", "quiet")]),
    ]);
  }
  if (name === "join-words") {
    return group("Use your recovery words", [
      field("Name this computer", "Harbor desk"),
      help("Type or paste the 12 recovery words. Numbers and capitals are fine."),
      field("Recovery words", WORDS.join(" "), true),
      help("Pages already on this computer will be combined with your synced pages. Nothing is deleted."),
      actions([btn("Open my pages", "primary"), btn("Back", "quiet")]),
    ]);
  }
  if (name === "join-wait") {
    return group("Approve on your other device", [
      help("The same six numbers should be on that phone or computer. Tap Yes there only if they match."),
      code("482 916"),
      help("Waiting for that phone or computer."),
      actions([btn("Cancel", "quiet")]),
    ]);
  }
  if (name === "approve" || name === "approve-timeout") {
    const kids = [
      help("It wants to open your pages. The same six numbers and the same four words should be on that screen. Tap Yes, approve it only if they match."),
      code("482 916"),
      el("p", { class: "sync-mock-code", text: "oats  ferry  bread  cake" }),
    ];
    if (name === "approve-timeout") {
      kids.push(el("p", { class: "sync-err", role: "alert", text: "That took too long. Your pages are still here." }));
    }
    kids.push(actions([btn("Yes, approve it", "primary"), btn("No", "quiet")]));
    return group("Is this Harbor phone?", kids);
  }
  if (name === "devices" || name === "list-changed") {
    const wrap = el("div", {});
    if (name === "list-changed") {
      wrap.append(group("List changed", [
        el("button", { type: "button", class: "sync-mock-notice", text: "Harbor phone is now on the list." }),
        help("Sync is still on. This note does not stop pages from saving."),
        actions([btn("Got it", "quiet")]),
      ]));
    }
    wrap.append(group("My pages", [
      help("Live. Pages keep saving while this list is open."),
      device("Harbor desk", "this computer", false),
      device("Harbor phone", "phone", true),
      actions([btn("Add a phone or computer", "primary"), btn("Turn sync off", "quiet")]),
    ]));
    return wrap;
  }
  if (name === "remove") {
    return group("Remove Harbor phone?", [
      help("It stops getting new pages. Pages it already saved stay readable on it. New edits stay with the phones and computers that are still linked."),
      actions([btn("Remove", "danger"), btn("Keep it", "quiet")]),
    ]);
  }
  if (name === "paused") {
    return group("Sync is paused", [
      help("This computer is not on the list, so new pages stay here. Your pages are still here. The Sync button in the sidebar stays."),
      actions([btn("Sync", "quiet"), btn("Join again", "primary"), btn("Check again", "quiet")]),
    ]);
  }
  if (name === "keyring") {
    return group("Set up your keyring", [
      help("The key store on this computer is not unlocked. Set it up once. After that, Omacus does not ask for a password when this app opens."),
      help("This leaves the key store without a password. Omacus can open it again without asking. A password on the key store asks again every time."),
      actions([btn("Set up the key store", "primary"), btn("Use a passphrase instead", "quiet")]),
    ]);
  }
  if (name === "off") {
    return group("Turn sync off on this computer?", [
      help("Your pages stay here. This computer leaves the list, so the other phones and computers stop showing it. If this is the last one, that is fine. The recovery words still open the pages."),
      actions([btn("Turn sync off", "primary"), btn("Not now", "quiet")]),
    ]);
  }
  return group("How sync works", [
    help("Pages are encrypted on this computer before they leave. The server keeps them, and it cannot read them."),
    help("You get 12 recovery words. They are the only way back in if you lose every phone and computer. Write them down. Omacus does not keep a copy."),
    help("You link another phone or computer with a code. After that, your pages stay in step, even when one of them is offline."),
    help("Pages already in this window can come with you. They stay as files on this computer."),
    actions([btn("Set up sync", "primary"), btn("Not now", "quiet")]),
  ]);
}

function entryLabel(name) {
  if (["intro", "fresh", "words", "check", "keyring"].includes(name)) return "Turn on sync";
  return "Sync";
}

function showEntry(label) {
  for (const node of document.querySelectorAll("[data-turn-on-sync]")) {
    node.hidden = false;
    if (node.classList.contains("sync-setup")) node.textContent = label;
    const span = node.querySelector("span");
    if (span) span.textContent = label;
    node.setAttribute("aria-label", label);
    if (node.title) node.title = label;
  }
}

export function paintSettingsMock(panel, name) {
  if (!panel) return false;
  const theme = new URLSearchParams(location.search).get("theme");
  if (theme === "light" || theme === "dusk" || theme === "dark" || theme === "paper") {
    document.documentElement.dataset.appTheme = theme;
  }
  panel.hidden = false;
  panel.classList.add("sync-mock");
  panel.classList.remove("sync-sheet");
  panel.replaceChildren(screen(name));
  showEntry(entryLabel(name));
  return true;
}

export function openMock(name, openSettings) {
  const root = document.querySelector("[data-settings-panel='settings']")?.parentElement;
  const tabs = document.querySelector(".settings-tabs");
  if (!root || !tabs) return;
  let tab = tabs.querySelector("[data-settings-tab='sync']");
  if (!tab) {
    tab = el("button", { type: "button", role: "tab", "data-settings-tab": "sync", "aria-selected": "false", text: "Sync" });
    tabs.append(tab);
  }
  let panel = root.querySelector("[data-settings-panel='sync']");
  if (!panel) {
    panel = el("div", { "data-settings-panel": "sync" });
    panel.className = "sync-panel";
    root.append(panel);
  }
  paintSettingsMock(panel, name);
  if (typeof openSettings === "function") openSettings("sync");
  paintSettingsMock(panel, name);
}
