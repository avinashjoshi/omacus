#!/usr/bin/env python3
"""The sync home screen renames every phone the same way."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sync = (ROOT / "site/app/sync.js").read_text()
names = (ROOT / "site/app/device-name.js").read_text()
css = (ROOT / "site/app/app.css").read_text()
sw = (ROOT / "site/app/sw.js").read_text()
build = (ROOT / ".github/scripts/build-web.sh").read_text()

start = sync.index("function drawLinked()")
end = sync.index("async function copyWords(", start)
home = sync[start:end]

errors = []
if "Name this" in home:
    errors.append("the device list still has its own name field")
if "for (const device of devices)" not in home or "[...state.devices]" not in home:
    errors.append("the list skips a device")
refresh = sync[sync.index("async function refreshDevices("):sync.index("function showRemoved(")]
former = refresh.split("list.former", 1)[1]
if "state.member = false" not in former or "else if (!state.removed && !state.member)" in refresh:
    errors.append("a phone that is not on the list can still say sync is on")
recover = sync[sync.index("async function startRecovery("):sync.index("function draw(")]
if "could not be added." not in recover or "await push()" not in recover:
    errors.append("opening with the words does not add this computer")
line = sync[sync.index("function deviceLine("):sync.index("function renameControl(")]
rename = sync[sync.index("function renameControl("):sync.index("function drawReady(")]
if 'setAttribute("data-self", "1")' not in line:
    errors.append("this phone is not marked")
if "renameControl(row, device)" not in home:
    errors.append("Rename does not cover every row")
if "beginRename(row, device)" not in rename:
    errors.append("Rename does not cover every row")
if "if (!device.self)" not in rename or "Remove" not in rename:
    errors.append("Remove is not limited to the other phones")
marker = '.sync-device[data-self="1"]'
if marker not in css:
    errors.append("this phone's row is not styled")
else:
    rule = css[css.index(marker):css.index(marker) + 500]
    if "var(--app-def)" not in rule:
        errors.append("this phone's row does not use the green dot")

off = sync.index("function deviceNameControl()")
join = sync.index("function drawJoinType()")
if "Name this" not in sync[off:join] or "deviceNameControl()" not in sync[sync.index("function drawOff()"):join]:
    errors.append("starting sync lost the name field")
if 'from "./device-name.js"' not in sync:
    errors.append("the name guess left the shared helper")
if "omacus-desktop" not in names:
    errors.append("the desktop app is unnamed")
if "omacus-test" in names:
    errors.append("the retired test app still names itself")
if "webkitMajor" in names or ">= 600" in names:
    errors.append("Chrome with WebKit is still called omacus Test")
wait = sync[sync.index("async function waitForApproval("):sync.index("async function acceptJoin(")]
accept = sync[sync.index("async function acceptJoin("):sync.index("async function approve(")]
if "approvalPlan" in wait:
    errors.append("the waiting screen accepts the list before the numbers are checked")
if "confirmLink" not in accept or "approvalPlan" not in accept or "These match" not in sync:
    errors.append("These match does not check the numbers before opening")
if '"device-name.js"' not in sw:
    errors.append("the offline shell missed the name helper")
if "sync-test.omacus.com" in sw or "omacus-sync-test" in sw:
    errors.append("the offline shell still names the retired sync host")
if "site/app/device-name.js" not in build or "./device-name.js?v=" not in build:
    errors.append("the published app missed the name helper")
version_js = (ROOT / "site/app/version.js").read_text()
shell_version = version_js.split('APP_VERSION = "', 1)[-1].split('"', 1)[0]
if f'const APP_VERSION = "{shell_version}"' not in sw or "omacus-app-${APP_VERSION}" not in sw:
    errors.append("the app cache is not named from the release version")
if '"sync-link.mjs"' not in sw:
    errors.append("the recovery link is not in the offline shell")
if "site/app/sync-link.mjs" not in build or "./sync-link.mjs?v=" not in build:
    errors.append("the published app missed the recovery link")
if "omacus-sync-web" not in build or "omacus_sync_web.js" not in build or "omacus_sync_web_bg.wasm" not in build:
    errors.append("the published build does not ship the sync engine")
if 'test -s "$dest/omacus_sync_web.js"' not in build or 'test -s "$dest/omacus_sync_web_bg.wasm"' not in build:
    errors.append("the published build keeps going when the sync engine is missing")
if 'from "./sync-vaults.mjs"' not in sync or '"sync-vaults.mjs"' not in sw:
    errors.append("the vault list is not in the app")
if "site/app/sync-vaults.mjs" not in build or "./sync-vaults.mjs?v=" not in build:
    errors.append("the published app missed the vault list")
calm = sync[sync.index("function calm("):sync.index("function armSaving(")]
follow = sync[sync.index("async function follow("):sync.index("function nextPoll(")]
notice = sync[sync.index("async function noticeJoin("):sync.index("async function follow(")]
if "findRequest()" not in notice:
    errors.append("a waiting phone is not noticed")
if "noticeJoin()" not in follow[follow.index("} catch"):]:
    errors.append("a save error stops this phone from seeing a join")
plain = sync[sync.index("function plainError("):sync.index("function joinError(")]
join_wording = sync[sync.index("function joinError("):sync.index("async function noteLocalPages(")]
if "/no pages/i" in plain:
    errors.append("a failed save says the code did not work")
if "/no pages/i" not in join_wording:
    errors.append("a bad code tells the joining phone there are no pages")
caught = follow[follow.index("} catch"):]
if caught.find("that code didn't work") < 0 or caught.find("saveError") < caught.find("that code didn't work"):
    errors.append("a save shows the joining phone's code error")
if "!state.member" in calm or "else if (!state.member)" in follow:
    errors.append("a phone that is not on the list says Sync paused")
if "setInterval(() => tick(), 4000)" in sync or "pollWait" not in sync:
    errors.append("sync still polls every 4 seconds while it is quiet")
join_src = sync[sync.index("async function submitJoin("):sync.index("function parseJoin(")]
if "typedJoinName()" not in join_src or 'name: ""' in join_src:
    errors.append("the typed name is not sent with the join")
if "joinError(" not in join_src:
    errors.append("the join screen can say there are no pages")
if "const APPROVAL_POLL_MS = 4000" not in sync or "setTimeout(waitForApproval, 1000)" in wait:
    errors.append("approval still polls every second")
if "const JOIN_WAIT_MS = 2000" not in sync or "setTimeout(waitForApproval, JOIN_WAIT_MS)" not in wait:
    errors.append("the joining phone does not check every 2 seconds")
invite = sync[sync.index("async function startInvite("):sync.index("async function submitJoin(")]
watch_invite = sync[sync.index("async function runInviteWatch("):sync.index("function applySharedRequest(")]
if "schedule()" not in invite or "const INVITE_POLL_MS = 2000" not in sync:
    errors.append("a new invite waits for the quiet poll")
if "findRequest(INVITE_POLL_MS)" not in watch_invite or "ifTabLeader" in watch_invite or "readMailbox" in watch_invite:
    errors.append("the invite check is stuck on the leader or reads the whole mailbox")
if "INVITE_POLL_HIDDEN_MS = 3000" not in sync or "inviteBackoff >= 4000" not in watch_invite:
    errors.append("a hidden invite or a busy server does not slow the join check")
if "joinWaits || 0) < 6" in wait or "joinStillOpen" not in wait:
    errors.append("approval gives up before the code expires")
if "adoptAskedName(askedCode, askedName)" not in sync:
    errors.append("approving drops the typed name")
if "MAILBOX_TRIES" not in wait or "error.status === 404" not in wait:
    errors.append("a missing list after approval ends the join")
if "done: true" not in wait or wait.find('state.screen = "join-confirm"') < wait.find("done: true"):
    errors.append("the number check comes before the phone can open the pages")
approve = sync[sync.index("async function approvePending("):sync.index("async function decline(")]
if "record: sealed.record" not in approve or "releaseJoin(" not in approve:
    errors.append("approval does not publish the list, or a failed join leaves the row")
watch = sync[sync.index("function watchApproval("):sync.index("function draw(")]
if 'state.screen = "linked"' not in watch:
    errors.append("the approve prompt stays after the phone stops waiting")
if "state.invite = null" in watch:
    errors.append("a phone that stops waiting takes the code with it")
if 'setChip("Device waiting", "ask")' not in sync or "omacusShowJoin" not in sync:
    errors.append("a waiting phone stays inside Sync")

if errors:
    raise SystemExit("\n".join(errors))
print("sync devices ok")
