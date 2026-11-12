# How Unity Hub works (behaviour notes for a clean-room reimplementation)

Observed on macOS (Apple Silicon), Unity Hub **3.22.6**, 2026-11-08.

These notes record only **behaviour and interoperable data formats**: files on disk,
public HTTP endpoints, and how editors are launched. They do not contain any of Unity
Hub's source code, and none of it should be copied into this project. Licensing and
sign-in internals are deliberately out of scope.

---

## 1. Hub data directory

`~/Library/Application Support/UnityHub/`

| File | Purpose | Format |
|---|---|---|
| `projects-v1.json` | Project list (the Hub also mirrors it into `hub.db`) | `{"schema_version":"v1","data":{"<abs path>": Project}}` |
| `hub.db` | SQLite: `projects(path TEXT PK, data TEXT JSON, updated_at INT)`, `meta(key,value)` | SQLite (WAL) |
| `editors-v2.json` | Editors added manually from outside the install folder | `{"schema_version":"2","data":[]}` |
| `excludedEditorFolders.json` | Editors the user removed from the list | `{"schema_version":"1","data":[]}` |
| `secondaryInstallPath.json` | Custom editor install root. `""` means the default | JSON string |
| `projectDir.json` | Folder last used for a new project, offered next time | `{"directoryPath": "..."}` |
| `Templates/` | Downloaded project templates and their `manifest.json` (see §10) | `.tgz`, JSON |
| `user-settings.json` | e.g. `download.maxConcurrentDownloads`, theme, tray | JSON |
| `modulesPreference.json` | Modules ticked last time | `{"selected":[ids],"deselected":[]}` |
| `install-state/install-state.db` | SQLite download/install queue (see §4) | SQLite |
| `releases.json` | Old cached release list (legacy) | JSON |
| `accounts.db`, `Cookies`, … | Auth/session. **Do not read or touch** | — |

### Project entry (`projects-v1.json` → `data[path]`)

```jsonc
{
  "title": "BlastBlockPuzzle",          // folder name, unless hasCustomDisplayName
  "path": "/abs/path",
  "containingFolderPath": "/abs",
  "version": "6000.0.72f1",             // from ProjectSettings/ProjectVersion.txt
  "changeset": "b731fd3ae857",          // revision hash, also from ProjectVersion.txt
  "architecture": "arm64",
  "lastModified": 1793927837255,        // ms since epoch
  "isFavorite": false,
  "isCustomEditor": false,
  "renderPipeline": "BUILT_IN",
  "vcsProvider": "github",              // optional
  "cloudProjectId": "...", "organizationId": "...", "cloudEnabled": true, // optional, cloud
  "sourceFingerprint": "..."            // Hub-internal change-detection cache; can be ignored
}
```

`ProjectSettings/ProjectVersion.txt` looks like:

```
m_EditorVersion: 6000.0.72f1
m_EditorVersionWithRevision: 6000.0.72f1 (b731fd3ae857)
```

## 2. Installed editors

Default root: `/Applications/Unity/Hub/Editor/<version>/`

```
<version>/
├── Unity.app                 # the editor; binary at Unity.app/Contents/MacOS/Unity
├── Unity Bug Reporter.app
├── PlaybackEngines/          # installed platform modules (iOSSupport, AndroidPlayer, …)
├── modules.json              # every module available for this version + install state
└── metadata.hub.json         # {"productName","architecture":"arm64","isLTS",...}
```

Editors are discovered by scanning the default root plus `secondaryInstallPath`
and adding `editors-v2.json`. The version string comes from the folder name, or more
reliably from `Unity.app/Contents/Info.plist` (`CFBundleVersion`).
Architecture comes from `metadata.hub.json`, or from `lipo -archs` on the binary.

## 3. Release catalogue (public API)

Documented public endpoint, no authentication needed:

```
GET https://services.api.unity.com/unity/editor/release/v1/releases
    ?platform=MAC_OS&architecture=ARM64&stream=LTS|TECH|BETA|ALPHA&limit=N&offset=M
    &version=<prefix>
```

Response: `{ total, results: [Release] }`, where each `Release` has
`version`, `releaseDate`, `stream`, `shortRevision`, `recommended`,
`unityHubDeepLink` (`unityhub://<version>/<revision>`), and `downloads[]`. Each download has:

- `url` (the editor `.pkg`), `type: "PKG"`, `integrity` (`md5-<base64>`),
  `downloadSize`/`installedSize`, and `modules[]`
- each module has `id`, `slug`, `name`, `category`, `url`, `type` (`PKG`, `ZIP`, `DMG`, …),
  `destination`, `required`, `hidden`, `preSelected`, `eula[]`, `subModules[]`

Hub 3.22 itself also polls `https://public-cdn.cloud.unity3d.com/hub/prod/releases-silicon.json`.
The services API above is the supported, documented one, so we use that.

Notes from using it:
- Leaving out `architecture` returns every macOS release with all its builds; releases
  before 2021 (and their 2025 security re-releases) only have `X86_64` builds.
- Leaving out `stream` searches every stream; `version` is a prefix match (`2022.3`,
  `2022.3.1` → 2022.3.1x). Queries that match nothing return `total: 0`.
- Releases carry `hasSupportEnded` and `endOfLifeDate` (e.g. 2022.3 LTS: ended 2026-05-08;
  6000.4/6000.5 Supported: ended once 6000.6 shipped). Only one release is `recommended`
  (the newest Supported). The Hub's "Official releases" tab matches: the newest release of
  each Supported and LTS line whose support has not ended (6000.6.8f1 Supported +
  Recommended, 6000.3.28f1 LTS, 6000.0.85f2 LTS on 2026-11-08).
- Sizes are usually whole numbers but some are fractional (`{"value": 658719121.2}`), and
  some fields are `null` rather than missing (`"eula": null`).

## 4. Download / install pipeline

The `install-state.db` schema shows the states the Hub uses:

- `downloads(uid, version, arch, modules_json, state ∈ {queued, downloading, complete, failed, cancelled}, bytes_total, bytes_done, pid, started_at, updated_at, error_code, error_msg)`
- `installs(... phase ∈ {queued, installing})`: **only one install runs at a time**
  (`CHECK(rowid = 1)`), while several downloads can run in parallel (default 2).

Steps:
1. Download the editor `.pkg` and verify the md5 `integrity`.
2. Install it into `<root>/<version>/` (or `<root>/<version>-<arch>` when that folder is
   taken). The Hub expands the pkg payload itself (xar, then the payload archive), so no
   admin rights are needed; it runs `installer` only for packages without a destination,
   such as Visual Studio.
3. For each module, download and install by `type` into `destination`. Destinations use
   the placeholder `{UNITY_PATH}` for the editor's install folder, e.g.
   `{UNITY_PATH}/PlaybackEngines` for iOS Build Support. Some tools go elsewhere, e.g.
   Visual Studio Code goes to `/Applications`.
4. Write `modules.json` and `metadata.hub.json` so the official Hub still recognises the install.
   On install the Hub writes the whole flattened module list with `selected: false`, then
   flips `selected` to true for each module that installs. Module PKGs are unpacked into
   `destination`, ZIPs unzipped, `.po` files copied, DMGs mounted and copied; then
   `renameFrom` → `renameTo` is applied.

### Modules of an installed editor (observed with REA, Hub 3.22.6)

- **Installed flag:** `selected: true` in `<editor>/modules.json`. `isInstalled` is derived
  when the list is read (it is only true on its own for Visual Studio Code, which the Hub
  finds in `/Applications`). For editors it installed, the Hub trusts `modules.json` and
  does not check the disk. For editors added with "Locate" it probes each module's
  destination once and then writes `modules.json` / `metadata.hub.json`.
- **Installs page chips:** one per build platform, from a folder scan of
  `<editor>/PlaybackEngines` and `Unity.app/Contents/PlaybackEngines` (so the built-in Mac
  platform always shows), not from `modules.json`.
- **Add modules dialog:** lists modules that are not hidden, grouped by category, children
  nested one level under their parent (Android → OpenJDK, Android SDK & NDK Tools). Hidden
  children (NDK, CMake, SDK platforms…) come along with their parent. Ticking a parent ticks
  its children; ticking a child ticks its parent; children can be unticked. Installed rows
  show "Installed" instead of a checkbox. The dialog shows required and available disk space
  and blocks installing when space is short.
- **License terms:** each selected module with `eula` gets its own page with an "I have read
  and agree" checkbox; Install stays disabled until every page is accepted. Acceptance is not
  stored.
- **Removing (Hub 3.22):** offered only for modules whose files are inside the editor folder
  (not Visual Studio Code). Deletes the module's folder (`PlaybackEngines/<Platform>`,
  standalone `Variations`, a language's `.po` file) and sets `selected: false`.
- **Intel editors:** the main list asks for Apple Silicon builds only; the archive offers
  Intel builds for versions without one, labelled "Intel".

### How UniHub does it

UniHub drives the official Unity CLI instead of reimplementing the pipeline:
`unity install <version> --architecture <arch> --no-child-modules --module <ids…>` and
`unity install-modules --editor-version <version> --no-child-modules --module <ids…>`, both
with `--format ndjson`, which prints `{"type":"progress","pct","msg","phase"}` lines and a
final `{"type":"result","success","errors"}`. With `--no-child-modules` only the visible
children the user kept ticked are installed; their hidden dependencies are added by the CLI.
`--accept-eula` is passed only after the user ticked every license checkbox. The CLI skips
unknown module ids with a plain-text "Missing module" note, so UniHub treats that as a
failure. Removing uses `unity editor module remove <version> --module <ids…> --yes`, and
uninstalling `unity uninstall <version> --yes`. `--download-directory <root>` is passed
when UniHub's install folder differs from the CLI's (`unity install-path`). The CLI keeps
`modules.json` up to date, and waits while another install (its own or the Hub's) runs.

## 5. Launching an editor

```
<root>/<version>/Unity.app/Contents/MacOS/Unity -projectPath <path> [-buildTarget <t>]
```

When the official Hub launches the editor it also passes Hub-session and IPC flags.
These belong to sign-in and licensing, so **we do not replicate them**. The editor gets
its license from the **Unity Licensing Client**. UniHub activates and returns licenses
through the official Unity CLI (§6), which manages that client.

## 6. The official Unity CLI (`unity`)

Hub 3.22 ships alongside Unity's command-line tool and now does much of its work through it.

- **Install:** `curl -fsSL https://public-cdn.cloud.unity3d.com/hub/prod/cli/install.sh | bash`
  (documented at <https://docs.unity3d.com/hub/unity-cli>). `UNITY_CLI_CHANNEL=beta` selects
  the beta channel, and the script falls back to beta while there is no stable release. It
  checks the binary's SHA-256 against the channel manifest, installs `~/.unity/bin/unity`,
  writes `~/.unity/env`, and sources it from `~/.zshrc` / `~/.bashrc`. It is also published
  as Homebrew `unity-cli` and npm `@unity/cli`.
- The Hub records the install in `cli-install.json` (`path`, `version`, `layout`) and
  auto-installs it once (`cliAutoInstallAttempted` in `user-settings.json`).
- **Machine-readable output:** every command accepts `--json` and prints one envelope,
  `{ "success", "command", "data", "errors": [{ "code", "message" }], "warnings" }`.
  Exit code 3 / `AUTH_ERROR` means "not signed in". `unity commands --json` lists every command.
- Commands UniHub uses:

| Need | Command |
|---|---|
| Version | `unity version` |
| Sign in (opens the browser) | `unity auth login` |
| Sign-in state | `unity auth status` → `loggedIn`, `sessionState`, `mode`, `user {name, email}` |
| Accounts | `unity auth accounts`, `unity auth switch <id>`, `unity auth logout [--yes] [<id>]` |
| Licenses | `unity license status`, `license activate --personal --accept-eula`, `license return --yes` |
| Tools | `unity plugin [--refresh]`, `plugin install <id> [--accept-eula]`, `plugin upgrade <id>`, `plugin remove <id> --yes` |
| CLI updates | `unity self-update --check`, `self-update --yes [--channel beta]`, `self-uninstall --yes` |

`unity plugin` lists managed components such as `plastic` (Unity Version Control `cm`),
`licensingClient` (runtime used for licensing), `ugs`, `ai` and `transform`, with
`installed`, `resolvedVersion`, `pinnedVersion`, `newestVersion` and `blockedByCliVersion`.

## 7. Sign-in

The Hub runs a local **auth broker** that shares the session with plugins. Its log shows
it only accepts peers code-signed by Unity (`peer-auth mode=enforce`), and rejects
unidentified callers. A third-party app therefore cannot and should not talk to it.
Tokens live in the macOS keychain (`credentialSource: keyring`).

UniHub signs in only by running `unity auth login`, and reads state with
`unity auth status`. It never reads, stores or passes Unity tokens.

## 8. Preferences

REA's static analysis of the Hub's preference-ID module lists these sections and options.
UniHub follows the ones that apply to it.

| Hub section | Options |
|---|---|
| Appearance | theme, language, minimise to tray, hide menu-bar icon |
| Locations | editor install location, download location |
| Projects | default project location, ask before removing a project, "default project name" (despite the name, this switches the project list between folder names and product names, `projects.showProductNames`; it is not the name suggested for new projects) |
| Editors | ask before uninstall, reset dismissed warnings |
| Notifications | enable, show pop-ups, silent |
| Deep links | always forward to a single editor |
| Privacy | analytics opt-in |
| Advanced | release channel, update server |
| AI | AI features |

Accounts and Licenses are separate pages in the Hub. Download concurrency is
`download.maxConcurrentDownloads` in `user-settings.json` (default 2).

## 9. Project details and source control

From REA's analysis of the Hub's project model, its locale strings, and the Unity CLI:

- **Details:** each project row has an "Expand project details" arrow (plus "Expand all
  project details"). The expanded area shows Editor version (with "Change Editor version"),
  Render pipeline (Built-in / URP / HDRP / SRP), Current platform, Size on disk, Tags,
  Cloud project and organisation, and the **source control repository**: Provider,
  Repository, `{provider}` organisation, Current branch, "View on {provider}", "Copy link",
  "Disconnect from {provider}" and remote repository size.
- **Project model fields:** `vcsData` (`provider`, `url`, `vcsRoot`,
  `projectPathInsideRepository`, `hasRemote`, remotes), `vcsProvider`,
  `vcsConfigurationPath` (`.git/config`), `repositoryName`, `buildTarget`, `size`, `tags`,
  `cloudProjectId`, `organizationId`/`organizationName`, `renderPipeline`.
- **Providers:** GitHub, GitLab, Perforce and Unity Version Control. The table can show and
  sort by the current branch.
- **Render pipeline:** the Hub reads `m_CustomRenderPipeline` in
  `ProjectSettings/GraphicsSettings.asset` and the per-quality `customRenderPipeline` in
  `QualitySettings.asset`. A `{fileID: 0}` reference means Built-in; otherwise the
  pipeline asset's script tells URP from HDRP.
- **Current platform:** `m_ActiveBuildTarget` in `Library/EditorUserBuildSettings.asset`,
  a *binary* Unity serialized file (format 22), which the Hub parses itself.
- **Unity CLI equivalents:** `unity projects info <path> --json` returns `buildTarget`,
  `renderPipeline`, `scriptingBackend`, `packages`, `vcsProvider`, `repositoryName` and cloud
  fields; `unity projects size <path> --json` returns `total` and a per-folder `breakdown`;
  `unity vcs status <path> --json` returns branch and change counts.

UniHub reads `.git/HEAD` and `.git/config` (and `.plastic/plastic.selector`) itself for
the table, runs `git` for the last commit, changes and upstream distance, and uses the
Unity CLI for the build target and size when it is installed. Web links are built only
for GitHub and GitLab, whose URL layouts are known; credentials in HTTPS remotes are
stripped before display. UniHub does not implement the Hub's GitHub/GitLab OAuth
connection ("Disconnect from {provider}").

## 10. Creating a project from a template

Observed with REA (Hub 3.22.6 main-process modules, its logs of two creations, and the
public template catalogue), 2026-11-09.

### Templates

- **Built-in:** every `*.tgz` in `<Unity.app>/Contents/Resources/PackageManager/ProjectTemplates`
  (the `libcache` folder there is not a template). A template is a UPM package: the
  archive holds `package/package.json` (`name`, `displayName`, `version`, `description`,
  `type: "template"`, `dependencies`) and the project itself under `package/ProjectData~/`
  (`Assets`, `Packages`, `ProjectSettings`). The file name can differ from the package
  name (`com.unity.template.3d-cross-platform-17.0.12.tgz` holds
  `com.unity.template.urp-blank`), so the Hub reads `package.json`. Built-ins are ready to use.
- **Downloaded:** `~/Library/Application Support/UnityHub/Templates/<name>-<version>.tgz`,
  recorded per editor in `manifest.json`:
  `{"6000.0.85f1": {"dependencies": {"com.unity.template.vr": "9.2.3"}}}`. A downloaded
  template is offered only to the editors listed for it.
- **Catalogue:** a GraphQL endpoint (`live-platform-api.prd.ld.unity3d.com/graphql`) queried
  with the editor's `<major>.<minor>`, no sign-in needed. Each template has a display name,
  description, type (`CORE`, `SAMPLE`, `LEARNING`), `isNew`, render pipeline, build
  platforms, icon and preview image URLs, and versions newest first, each with a size and a
  `tarball` (`url` on `download.packages.unity.com`, `integrity`). The integrity is
  `sha1-<base64>`, where the base64 holds the hex digest (npm's form, the raw 20 bytes, also
  occurs); it is checked against the SHA-1 of the downloaded file.
- **Merging:** the newest catalogue version for the editor is compared with the copy on
  disk: no copy means "Download template"; the same version uses the file with the
  catalogue's display data; a newer catalogue version is offered as an upgrade; a newer
  local copy (a built-in 17.0.12 against a catalogue 17.0.9) wins.
- The dialog has All, New, Core, Sample and Learning tabs, plus Custom for templates the user
  made. Ready templates show "Create project"; the others "Download template" first
  (with an age or terms step when the catalogue asks for one).
- The Unity CLI gives the same merged list: `unity templates --editor-version <v> --json`
  returns `items` with `name`, `displayName`, `type`, `version`, `status` (`READY`,
  `DOWNLOADABLE`), `tar` (the archive on disk), `remoteTar`, `remoteVersion`, `integrity`,
  `size`/`remoteSize`, `renderPipeline`, `buildPlatforms`, `dynamicIcon`, `previewImage`,
  `isNew` and `isUserGenerated`.

### Name and location

- Suggested name: "My project", or "My project (n)" with `n` one above the highest number
  already used in the folder (names compared without regard to case).
- Default location: `projectDir.json`, else the home folder; the Hub saves the folder used
  after each creation.
- The name must work on Windows too: not empty, at most 223 characters, none of
  `< > : " / \ | ? * ` %` or control characters, no leading space, no trailing space or dot,
  and not a reserved Windows name (`con`, `prn`, `aux`, `nul`, `com1`–`com9`, `lpt1`–`lpt9`).
- The project folder must **not exist** yet, not even empty, and its parent must be
  readable and writable.

### Launching the editor

The Hub does not use the Unity CLI for this. It starts the editor directly, as a normal
(not batch-mode) launch:

```
<Unity.app>/Contents/MacOS/Unity -createProject <parent>/<name> \
    -cloneFromTemplate <template .tgz> \
    -cloudOrganization <id or ""> -cloudProject <id or ""> -cloudEnvironment production
```

- **The editor creates the folder**, unpacks the template, resolves packages and opens the
  project.
- Without Unity Cloud both cloud ids are empty strings. With Unity Cloud or Unity Version
  Control the Hub first creates the cloud project or repository through Unity's services
  and passes its id. The Hub also adds its session, IPC and access-token flags; those belong
  to sign-in and are not replicated.
- For an editor without a `ProjectTemplates` folder, the Hub falls back to
  `-projectTemplate 0|1` (old built-in 3D/2D).
- The project is added to `projects-v1.json` right after the editor starts (title, path,
  version, architecture, now as `lastModified`), without waiting for the editor. It is taken
  off again if the editor fails before the folder is a Unity project. Other fields are
  filled in by later refreshes.
- Custom templates may ask for `%PROJECT_NAME%` to be replaced in some of their files
  (`hasProjectNamePlaceholder`, `rootFiles` in `package.json`); the Hub repacks such a
  template before launching.
- `unity projects create` does the same in batch mode (`-batchmode -nographics -quit`),
  downloads templates itself, and adds extra setup unless `--bare`.

### How UniHub does it

- **Templates:** `unity templates --editor-version <v> --json` when the Unity CLI is
  installed; without it, the editor's built-in templates and the ones downloaded for that
  editor into UniHub's `templates/` folder or the Hub's `Templates/` folder (read-only),
  read from their `package.json`. Archives the CLI already describes are not opened.
- **Downloading:** a template that is not on disk is downloaded from its `remoteTar` URL
  into `~/Library/Application Support/UniHub/templates/<name>-<version>.tgz`, checked
  against its `integrity`, and recorded in a `manifest.json` of the same shape as the Hub's.
  Finding the download URL needs the Unity CLI.
- **Creating:** the same rules for the name and folder, then the editor is started with
  `-createProject`, `-cloneFromTemplate` and the empty cloud ids, as above. The project is
  listed in UniHub right away (UniHub still never writes the Hub's files, so the Hub sees
  it only after an import into its own list) and the job finishes once the editor has
  written `ProjectSettings/ProjectVersion.txt`. If the editor quits before that, the
  project is taken off the list. Cancelling stops the download or the editor.
- Not done: Unity Cloud and Unity Version Control connection for new projects, the
  `%PROJECT_NAME%` replacement for custom templates, and age or terms steps.

## 11. Scope of UniHub

In scope: project list (compatible with `projects-v1.json`, read-only), creating projects
from templates, editor discovery, release browsing, launching, settings, a CLI,
connecting projects to Unity Cloud (§12), and account, license and tool management
**through the official Unity CLI**.

Out of scope: reimplementing Unity's sign-in, token handling or licensing protocols,
other cloud/organisation features (creating cloud projects, members, services), Learn,
Community.

## 12. Connecting a project to Unity Cloud

Observed with REA (Hub 3.22.6 main-process and renderer modules, the Hub's logs, its
`hub.db` read-only), Unity CLI 1.0.0-beta.17 and throwaway test projects, 2026-11-12.

### What a connection is

- Three fields of the PlayerSettings block in `ProjectSettings/ProjectSettings.asset`:
  `cloudProjectId`, `organizationId` and `projectName` (the cloud project's name). The Hub
  counts a project as connected when both ids are non-empty. An unset field is written
  `key: ` (a trailing space after the colon). `cloudEnabled` is not touched by connecting
  or disconnecting.
- `organizationId` is found in three forms: the organization's numeric ("genesis") id, as
  the Hub writes it; its UUID, as the Unity CLI writes it; and its name, as the editor
  writes it after it opens the project. Each is accepted.

### Unity Hub

- **Connect to Unity Cloud** (project row menu, project details): refused while the
  project is open in an editor. The dialog lists the account's organizations and, for the
  chosen one, its cloud projects (Unity's gateway API, by numeric organization id); it can
  also create a new cloud project with a typed name when the user may create projects in
  that organization. Linking itself makes no server call: the Hub checks that the cloud
  project id is a version 1–5 UUID, then replaces the three lines in place, keeping their
  indentation (`organizationId` = numeric id, `cloudProjectId`, `projectName` = cloud
  project name). The rest of the file is unchanged. It records `cloudProjectId`,
  `organizationId`, `genesisOrgId` and `projectName` on the project's list entry and shows
  a "connected" notification.
- **Disconnect**: after a confirmation, the three fields are emptied and the list entry's
  cloud ids cleared. For a project in Unity Version Control the Hub also disconnects the
  UVCS workspace.
- **Changes made outside the Hub**: the Hub re-reads `ProjectSettings.asset` when it
  refreshes (for example when its window gains focus). Ids that appear in the file are
  taken over (logged as "Project connected to Unity Cloud: <path>", with the
  notification). Ids that disappear are not: the Hub keeps the ids it holds in memory and
  saves them back to its list. While the Hub runs, its list therefore keeps showing a
  project as connected after a disconnect made elsewhere, including with
  `unity projects unlink cloud`. This only affects the Hub's list: opening a project from
  the Hub passes no cloud arguments, so the editor goes by the file.
- Hub 3.22.6 keeps its project list in `hub.db`; `projects-v1.json` was not updated by any
  of these operations.

### Unity CLI

- `unity cloud org --json` → `organizations[]` with `id` (UUID), `genesisId`, `name`,
  `role`, `isDefault`. `unity cloud project --cloud-org <id|name> --json` → `projects[]`
  with `id`, `name`, `organizationId`, `organizationGenesisId`, `archivedAt`, … (without
  `--cloud-org` and no default organization: `CLOUD_NEEDS_ACTIVE_ORG`).
- `unity projects link cloud <path> --cloud-org <org> --cloud-project <id|name>` and
  `unity projects unlink cloud <path>` work only for projects in the Hub's list; otherwise
  they fail with "No registered project found" (`PROJECT_NOT_FOUND`, or
  `PROJECTS_UNLINK_CLOUD_ERROR` for unlink). Link writes the organization's UUID and the
  *local* project's folder name as `projectName`, unlike the Hub, and updates the list
  entry; without a project it creates a new cloud project. Unlink restores the file
  byte for byte to its unset state and clears the list entry.

### How UniHub does it

- **Connect:** organizations and cloud projects come from the Unity CLI (the sign-in stays
  with it; archived cloud projects are left out). UniHub then writes the three fields
  itself, with the Hub's values: numeric organization id (the UUID when there is none) and
  the cloud project's name, single-quoted when YAML needs it. It keeps line endings, writes
  through a temporary file, and refuses while the project is open in Unity or when the file
  has no such fields. Checked against the running Hub: on its next refresh it logged the
  connection and its `hub.db` entry held the same ids and name as after a Hub connect.
- **Disconnect:** `unity projects unlink cloud` first, so the Hub's list entry is cleared
  as well, then the same edit directly when the fields are still set (projects the Hub does
  not list, or no CLI). The file ends up byte-identical to its unset state. The UVCS
  workspace is left alone. When the Hub is running, the confirmation says that its list
  may keep showing the project as connected.
- Not done: creating a new cloud project, and disconnecting Unity Version Control.
