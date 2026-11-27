# Data API for themes and plugins (design)

Status: **D0 (the store) and D3 (plugin access) are implemented** (`src/extras/data/`, plugin reference in
[plugins.md, "Data API"](plugins.md#reference), generated field list in [data-model.json](data-model.json)); the theme
side (D1, D2, D4–D7) is design. See [Implementation notes](#implementation-notes-d0--d3) at the end for what the code adds
to or decides beyond this design. This document extends [theming-engine.md](theming-engine.md): it
defines the data a theme or a plugin can read, how a data-only layout binds to it, the actions it can trigger, and the
permissions around both. The first theme built on it is "Night Chat", a server-bar layout (section 9).

## Summary

A theme can show **any data the official Webex app shows** and build its own views from it without native code. It
does that through four pieces:

1. **One data model** (`webox-data/1`): typed, versioned entities with stable ids (Me, Person, Presence, Space, Team,
   Message, Call, Meeting, ...) built from what Webox already keeps in memory. Section 2.
2. **Declarative bindings** for themes: named data sources with `where` / `sort` / `group` / `limit` / `search`, computed
   values, per-item templates, virtualised lists, and empty and loading states. Queries compile once and update
   incrementally. Section 3.
3. **Host methods** for native plugins: `data.query`, `data.get`, `data.subscribe` / `data.unsubscribe` with diff events.
   They use the same model and the same query language. Section 4.
4. **Actions** on the existing action bus (`App::dispatch`, theming engine "Behaviour and coherence" §3), with guards and
   a permission tier for each. Section 5.

Webex data never leaves the machine through this API. A theme has no network access, no file access and runs no code.
Section 6 covers permissions, section 7 images, section 8 the performance budget.

---

## 0. Two kinds of extension

Webox has exactly two kinds of extension. The API is built around the difference.

| | **Theme** | **Plugin** |
| --- | --- | --- |
| What it is | A data-only package: theme tokens, fonts, icons, images, layout trees, widget templates and data bindings. | Native code (`cdylib`, `docs/plugins.md`), for behaviour: message logger, auto-reply, MCP, video effects. |
| Runs | Inside Webox's engine. Nothing of the theme executes. | In the Webox process with Webox's rights. |
| Reads data | Through declarative bindings (section 3), only the scopes it declares and the user granted. | Through `data.query` / `data.get` / `data.subscribe` (section 4) and the existing host methods. Full access. |
| Acts | Through the action bus, from a user gesture on its widgets, within its action tier (section 5.2). | Through `ui.dispatch` and host methods. Full access, the same guards as built-in UI. |
| Network, files | None. Remote images only as static URLs fetched by Webox (section 7). | Anything (native code). |
| Install warning | No "native code" warning. The install screen lists data scopes, action tiers and image domains. | The existing "native code has full access" warning. |
| Settings tab | **Settings > Themes** | **Settings > Plugins** |
| Repository entry | `"kind": "theme"`, one `"any"` file (the zip) | `"kind": "plugin"` (default when absent), one file per platform |
| Validation | Strict schema + lint (`webox --lint-ui`); invalid input is refused as a whole. | ABI and contract checks, SHA-256 pinning (as today). |
| Updates | May update silently when the new version asks for **no more** scopes, tiers or image domains than were granted; otherwise the update waits for the user's approval of the new items. | As today: offered, never installed unasked; the warning stays. |

A plugin may **ship a theme part** (for example a panel template for its own UI, `<pkg>/<version>/ui/`). That part goes
through the same theme engine, validation and lint. Its bindings are evaluated by Webox, not by the plugin, so a theme
part of a plugin and a stand-alone theme behave the same. The plugin itself still has full access through its native
code, which is why it carries the warning.

Consequences:

- **Settings has two tabs.** "Themes" lists theme packages (picker, layout choice, per-package permissions, image
  domains, lint findings). "Plugins" keeps today's page for native plugins.
- **Repository index**: an entry gains an optional `kind: "theme" | "plugin"`. Absent means `"plugin"`, so existing
  indexes keep working. Older Webox versions skip theme entries because they have no platform file they know.
- **Permission model per kind**: themes have scopes and action tiers that the user grants (section 6). Plugins have none
  beyond the existing all-or-nothing install, but `data.*` still records the caller so the page can show "this plugin
  reads message content".

---

## 1. Inventory: what Webox has

Status legend:

- **A** (available now): in memory in Webox today; the data layer only has to expose it.
- **F** (fetchable): a request Webox already implements or whose shape is known; it has to be called (on demand or
  when a view needs it).
- **M** (missing): needs a Webex request Webox does not make yet (named where known), or Webex does not offer it.

All data arrives on the UI thread as `cloud::Resp` messages and is applied in `App::poll` (`src/app/mod.rs`). That one
place is where the data store (section 2.6) is kept up to date.

### 1.1 Me

| Data | Source | Status |
| --- | --- | --- |
| id, display name, e-mail | `Resp::Me(Person)` → `App.me` (`src/cloud/mod.rs` `Person`; `Session::me_person`, `src/cloud/session.rs`) | A |
| Avatar, cover picture | `Resp::Avatars` / `Resp::Cover` → `App.photos`, `App.cover` (`src/assets/photos/`) | A |
| My presence (status, last active, until, text) | `Resp::Presence` / `Resp::PresenceDetail` for my e-mail (`src/cloud/people/mod.rs`) | A |
| DND and its end; away / active choice | same; set with `Req::SetPresence` (`src/cloud/presence_set.rs`) | A |
| Custom status text and end | `settings["custom_status"]` (`src/ui/profile_menu.rs`). Local only, not sent to Webex. | A |
| Quiet hours | `settings["quiet_hours"]`, `quiet_start` / `quiet_end` / `quiet_day` | A |
| This device's registration | `Resp::Device` → `settings["device"]` | A |
| Audio input and output devices, ringtone device, volume | `media::audio::devices()`, `audio::Choice` (`src/media/audio.rs`) | A |
| Cameras | `src/media/camera/` | A |
| Feature toggles and client policies | `cloud::features` (`src/cloud/features.rs`) | A |
| Job title, department, phone, manager, company | not fetched (`src/ui/contacts_page.rs` header lists them as "not shown yet") | M (identity / people record of the user) |

### 1.2 Spaces

| Data | Source | Status |
| --- | --- | --- |
| id, title, direct (1:1), team, public, hidden | `cloud::Space` (`src/cloud/mod.rs`), `Resp::Spaces`, `Resp::List(ListPatch)` (`src/cloud/live_list/`) | A |
| Favourite | `Space.favorite`, `ListPatch::Favorite` | A |
| Unread (yes/no), last activity, last seen | `Space.unread`, `.last`, `.last_seen`, `ListPatch::Read` | A |
| Unread **count** | not in the list answer | M (Webex shows a dot, not a count; a count can be derived for loaded spaces only) |
| Mentions of me | live: `message.received.mentioned` (plugin event payload, computed in the UI); loaded messages: from the HTML mentions | A for new and loaded messages; **M** for a per-space mention flag after restart (the conversation list field for it is not traced) |
| Notification setting (global / all / mentions / off) | `spaces_ext::SpaceInfo.notify` (`src/cloud/spaces_ext/mod.rs`), `ListPatch::Info` / `NotifyTags` | A |
| Moderators, locked | `SpaceInfo.moderators`, `.locked` | A |
| Read-only: closed by policy, announcement mode | `Space.closed`, `Space.announcement` (`src/ui/closed.rs`) | A |
| Space picture, space cover | `Resp::SpaceAvatars`, `Resp::SpaceCovers` | A |
| Participants (first 10) | `Space.participants` (list request `participantsLimit=10`, `src/cloud/lists.rs`) | A |
| All participants, member count | `SpaceOp::People` → `SpaceEvent::People` (limit 500) | F |
| Meeting running in the space, its locus | `Space.meeting`, `Space.locus_url`, `ListPatch::Meeting` | A |
| Who is in that running meeting | `GET <locus_url>` (the locus Webox already parses for its own calls, `src/cloud/locus/`) | F |
| Pinned messages | popover exists with the empty state only (`src/ui/space_tabs/mod.rs`) | M (pinned-messages request not traced) |
| Typing | `Resp::Typing` (`src/cloud/typing.rs`) | A |
| Drafts | `App.drafts`, `settings["drafts"]` | A |
| Files and links of a space ("Content" tab) | derived from loaded messages (`src/ui/space_tabs/`) | A for loaded history; F for all (page back with `Req::MessagesBefore`) |

### 1.3 Teams

| Data | Source | Status |
| --- | --- | --- |
| id, name, colour | `cloud::Team`, `Resp::Teams` | A |
| General space, team url, archived | `spaces_ext::TeamInfo` | A |
| A team's spaces, including ones I have not joined | `SpaceOp::TeamSpaces` → `SpaceEvent::TeamSpaces` | F |
| Team members | `SpaceOp::People` on the general space | F |
| Team picture | Webex teams have a colour and initials, no picture | A (colour + initials) |

### 1.4 People

| Data | Source | Status |
| --- | --- | --- |
| id, name, e-mail | `cloud::Person`, `Resp::People` (people of loaded spaces) | A |
| Avatar, cover (people with a 1:1) | `Resp::Avatars`, `Resp::Covers` | A |
| Presence status: active, inactive, away, DND, busy, quiet, OOO, **call, meeting, presenting**, calendar | `Resp::Presence`; `people::normalize_status` / `state_status` | A |
| Last active | `Resp::Presence` → `photos.last_active` | A |
| Status end ("until 15:00"), custom status / OOO text | `Resp::PresenceDetail` → `photos.presence_info` | A |
| People outside loaded spaces (directory) | `Req::DirectorySearch` → `Resp::Directory`; their presence followed via `photos.outside_status` (`src/cloud/presence_sub.rs`) | A |
| Job title, department, phone, company, manager | not fetched | M (identity / people record) |
| **What** call or meeting another person is in, and **with whom** | Webex presence carries only the state | M, not offered by Webex (unsupported) |

### 1.5 Messages

| Data | Source | Status |
| --- | --- | --- |
| id, author, author e-mail, mine, time, date, `published_ms` | `cloud::Message` (`src/cloud/mod.rs`), `Resp::Messages` / `Resp::Incoming` | A |
| Plain text and rich HTML | `Message.text`, `Message.html` | A |
| Thread (parent root) | `Message.parent` | A |
| Reactions with reactors | `Message.reactions: Vec<Reaction>` with `Reactor` | A |
| Images, files, Giphy, link previews | `Message.images`, `.files` (`Attachment`), `.previews` (`LinkPreview`) | A |
| Edited | `Message.edited` | A |
| Deleted (tombstones), system lines, join/leave groups | `Message.notice`, `.group` (`src/cloud/activities/`) | A |
| Kept deleted messages, edit history (plugin) | `Message.deleted`, `.history`, `.current`, `.edit_tag` (`src/extras/plugins/kept.rs`) | A |
| Quotes | `<blockquote class='messageQuote'>` in `Message.html` | A |
| Flags | `Resp::Flags` / `Resp::Flagged` → `App.flags` (`src/cloud/msg_ops.rs`) | A |
| Read receipts ("seen by") | `Resp::ReadMarks` → `photos.read_marks` | A |
| Older history | `Req::MessagesBefore` → `Resp::OlderMessages` | F |
| Reminders, scheduled messages | local: `settings["message_reminders"]` (`src/ui/msg_remind.rs`), `settings["scheduled_messages"]` (`src/ui/schedule_send.rs`) | A |

### 1.6 Calls

| Data | Source | Status |
| --- | --- | --- |
| State (dialing, ringing, incoming, connecting, connected, ended), peer, direction | `locus::CallEvent` / `CallState` (`src/cloud/locus/mod.rs`) → `App.call` (`src/ui/call/mod.rs` `CallUi`) | A |
| Participants: name, e-mail, state, muted, hand raised, host, me | `locus::controls::Roster` / `Member` (`src/cloud/locus/controls.rs`) | A |
| My mute: local, remote, unmute allowed, asked to unmute | `locus::MuteInfo` / `SelfMute` | A |
| Video: wanted, sending, camera, size, fps, remote camera on | `locus::video::VideoInfo` (`src/cloud/locus/video.rs`) | A |
| Screen share: mine, the other side's (who) | `Resp::CallShare`, `Resp::RemoteShare` | A |
| Media statistics (RTT, loss, jitter, sizes per stream) | `media_stats::Snapshot` / `Flow` (`src/cloud/media_stats.rs`) | A |
| Call history (direction, missed, duration, peer) | `meetcall::CallRecord` via `Req::CallHistory` (`src/cloud/meetcall.rs`) | A (loaded when the Calls page opens; F otherwise) |
| Voicemail | the page exists (`src/ui/calls_page/voicemail.rs`), no cloud source | M (Webex Calling licence, `docs/calling-gap.md` §3) |

### 1.7 Meetings

| Data | Source | Status |
| --- | --- | --- |
| Running and starting-soon meetings: title, start, join link | `cloud::Meeting`, `Resp::Meetings` (loci poll, `src/cloud/polls.rs`) | A |
| Calendar: title, start, end, organiser, location, link, space, recurring, attendee count | `meetcall::CalEvent`, `Resp::Calendar` (live on `calendar.meeting.*`) | A |
| My personal room link | `Req::PersonalRoom` | F |
| Participants of a meeting I am not in | `GET <locus_url>` | F |

### 1.8 Contacts, search, notifications, settings, plugin host

| Data | Source | Status |
| --- | --- | --- |
| "Contacts" = people of my spaces; favourite people = favourite 1:1 spaces | `App.people`, `contacts_page::is_favorite_person` | A |
| The Webex contact list and contact groups | not fetched | M (contacts service) |
| Add a contact | not implemented (the button is left out on purpose, `src/ui/contacts_page.rs`) | M (contacts service write) |
| Local search: people, spaces, loaded messages | `src/ui/global_search.rs`, plugin `people.search` / `messages.search` | A |
| Directory search | `Req::DirectorySearch` | A |
| Server-side message search over all history | not implemented | M (Webex search service, not traced) |
| Pending toasts, unread total, taskbar badge | `platform::notify::Hub` (`src/platform/notify/mod.rs`) | A |
| A notification history / activity feed | Webex has none beyond unread state and missed calls | M (unsupported; derive from unread spaces + missed calls) |
| Webox settings (theme, density, zoom, shortcuts, call options, ...) | `App.settings` JSON, `src/shortcuts/` | A |
| App state: page, selected space, connection status, signed in | `App.tab`, `.selected`, `.status`, `.need_login` | A |
| Plugin list, enabled state, plugin settings, kept messages | `extras::plugins::Plugins`, `kept.rs` | A |

### 1.9 Count

| Status | Items |
| --- | --- |
| Available now (A) | 57 |
| Fetchable (F) | 9 |
| Missing (M) | 11 (of which 2 are not offered by Webex at all: what call or meeting another person is in and with whom, and a notification history; "pending contact requests" have no Webex counterpart either, see 9.7) |

The rows above with a split status count once under their weaker status.

---

## 2. Data model (`webox-data/1`)

### 2.1 Rules

- **Stable ids.** A person's id is the Webex user UUID (resolved through the session's participant-id → UUID map), or
  `email:<lower-case address>` while no UUID is known; the store merges the two when the UUID arrives and emits a
  `rekey` change. Spaces, teams, messages, meetings and calendar events use their Webex ids. A call uses its locus url.
  Ids are opaque strings to themes.
- **Times** are Unix milliseconds (`i64`, JSON number). Webox's ISO strings are converted once on arrival.
- **Images** are opaque handles (`ImageRef`, e.g. `"img:person:<id>"`). Only the `image` widget accepts them. They never
  contain a URL or a token (section 7).
- **Enums are open.** A theme must handle unknown values (they fall into `default` in a `switch`). New values may
  appear in any version.
- **Optional means absent.** A field that is not known is absent (`null` in plugin JSON), never an empty placeholder.
- **Additive only.** Within `webox-data/1`, fields, entities, enum values, relations and query functions are only added.
  Nothing is removed, renamed or retyped. A change of meaning creates a new name (`presence.state2`) or a new major
  version (`webox-data/2`), which is served alongside the old one for at least one minor release cycle.
- **Each entity carries `rev`** (a `u64` bumped on every change), so plugins can detect changes cheaply.
- **Packages declare** `"requires": { "data": "webox-data/1", "fields": ["space.mentioned"] }`. An unknown field is a
  lint error that names the Webox version that has it.

### 2.2 Entities

Notation: `name: type` per field; `?` = optional; `[T]` = list; `→ Entity` = a relation (an id, navigable in bindings).
Each field also lists its **scope** (section 6) in brackets.

```text
Me                                      [me.basic unless noted]
  id: string                            same id as the Person for me
  person: → Person
  device: string                        this device's id                                   [me.devices]
  audio: { inputs: [Device], outputs: [Device], input?: string, output?: string, ringtone?: string, volume: f32 }  [me.devices]
  cameras: [Device]                                                                       [me.devices]
  custom_status?: { text: string, until?: ms }                                            local, not sent to Webex
  quiet_hours?: { start: "HH:MM", end: "HH:MM", days: [u8] }
  dnd: bool                             derived: presence.state == "dnd"
  features: { <key>: bool | number | string }                                             [me.features]

Device
  id: string, name: string, kind: "audio_in" | "audio_out" | "camera", default: bool

Person                                  [people.basic]
  id: string
  name: string
  initials: string                      derived
  avatar?: ImageRef
  cover?: ImageRef
  email?: string                        [people.email] (my own e-mail is in me.basic)
  title?: string, department?: string, phone?: string   reserved, absent until fetched (M)
  presence: → Presence
  direct?: → Space                      the 1:1 space with this person, if any
  favorite: bool                        derived: direct?.favorite
  external: bool                        outside my org (when known)

Presence                                [people.presence]
  person: → Person
  state: "active" | "inactive" | "away" | "dnd" | "busy" | "quiet" | "ooo" | "call" | "meeting"
       | "presenting" | "calendar" | "unknown"
  online: bool                          derived: state in [active, busy, call, meeting, presenting, calendar]
  in_call: bool                         derived: state in [call, meeting, presenting]
  last_active?: ms
  until?: ms                            end of DND / meeting / OOO
  text?: string                         custom status or OOO text

Space                                   [spaces.list]
  id: string
  title: string
  kind: "direct" | "group"
  team?: → Team
  peer?: → Person                       direct spaces: the other person
  avatar?: ImageRef                     group picture; for direct spaces the peer's avatar
  cover?: ImageRef
  favorite: bool, hidden: bool, public: bool, archived: bool
  unread: bool
  mentioned?: bool                      a message I have not seen mentions me (see 1.2 for its limits)
  unread_count?: u32                    reserved (M); absent today
  last_activity: ms
  last_seen?: ms
  notify: "global" | "all" | "mentions" | "off"
  muted: bool                           derived: notify == "off"
  writable: bool                        derived: not closed, not announcement-blocked
  read_only_reason?: "closed" | "announcement"
  locked: bool
  moderators: [→ Person]
  members: [→ Person]                   the known ones (first 10 until loaded)
  member_count?: u32                    exact once the full list is loaded (F)
  meeting?: { active: bool, locus: string }        a meeting runs in this space
  typing: [→ Person]
  draft?: string                        [messages.content]
  last_message?: → Message              [messages.content]

Team                                    [spaces.list]
  id: string, name: string, color?: "#RRGGBB", initials: string
  general: → Space, archived: bool
  spaces: [→ Space]                     joined spaces of the team
  unread: bool                          derived: any joined space unread
  mentioned?: bool                      derived

Message                                 [messages.content; time/author/kind alone: messages.meta]
  id: string
  space: → Space
  author: → Person
  mine: bool
  at: ms
  kind: "message" | "system" | "deleted" | "join_leave"
  text: string, html?: string           html is sanitised; rendered only by the rich_text widget
  parent?: → Message                    thread root
  replies?: u32                         derived from loaded messages
  edited: bool
  history?: [[Span]]                    plugin-kept earlier texts
  reactions: [Reaction]
  files: [File]
  previews: [{ url: string, title: string, description: string, site: string, image?: ImageRef }]
  mentions_me: bool
  flagged: bool
  quote?: { author: string, text: string }
  seen_by: [→ Person]                   [messages.receipts]

Reaction
  name: string, count: u32, mine: bool, people: [→ Person]

File                                    [files]
  name: string, mime: string, size: u64, kind: "image" | "video" | "gif" | "file"
  thumb?: ImageRef, width?: u32, height?: u32

Call                                    [calls.state; stats in calls.stats]
  id: string                            locus url
  kind: "direct" | "space" | "meeting"
  state: "dialing" | "ringing" | "incoming" | "connecting" | "connected" | "ended"
  incoming: bool
  peer?: → Person                       1:1 calls
  space?: → Space
  started_at?: ms                       when connected
  participants: [Participant]
  me: { muted: bool, muted_by_host: bool, unmute_allowed: bool, asked_to_unmute: bool,
        video: bool, video_sending: bool, sharing: bool, hand: bool, host: bool }
  remote_share?: → Person
  stats?: { rtt_ms?: f32, audio: Flow, video: Flow, share: Flow }       [calls.stats]

Participant
  id: string, person?: → Person, name: string, state: "joined" | "ringing" | "left",
  muted: bool, hand: bool, host: bool, me: bool

CallRecord                              [calls.history]
  id: string, at: ms, ended_at: ms, secs: u32, outgoing: bool, missed: bool,
  peer?: → Person, name: string, space?: → Space

Meeting                                 [meetings.list]
  id: string, title: string, start: ms, end?: ms, organizer?: string, location?: string,
  join: JoinRef                         opaque; only `meeting.join` accepts it
  space?: → Space, recurring: bool, attendee_count?: u32
  running: bool                         derived from the loci list / space meeting state

Notification                            [notifications]
  id: string, kind: "message" | "mention" | "call" | "missed_call" | "meeting" | "reminder",
  at: ms, space?: → Space, person?: → Person, title: string, body?: string   body needs messages.content

Setting                                 [settings.read]
  key: string, value: bool | number | string, kind: "bool" | "number" | "choice" | "text"
  Only a whitelist of display settings (theme, density, zoom, rail_wide, compact_view, show_presence,
  msg_font_size, gif_autoplay, ...). Tokens, ids, paths and the e-mail are never exposed.

App                                     [always]
  page: "messaging" | "teams" | "contacts" | "calls" | "meetings" | "settings" | ...
  selected?: → Space, connected: bool, signed_in: bool, width_class: "narrow" | "medium" | "wide",
  focus_mode: bool, unread_total: u32, call_active: bool
```

`App` replaces nothing: it is the typed form of the theming engine's state keys (`nav.page`, `space.selected`,
`call.active`, ...), so a layout condition and a data binding read the same values.

### 2.3 Collections

Bindings and `data.query` start from a collection:

| Collection | Entity | Contents |
| --- | --- | --- |
| `people` | Person | everyone the store knows (space members, directory results kept, call participants) |
| `contacts` | Person | the people of my spaces (today's Contacts page set); later the Webex contact list |
| `spaces` | Space | my spaces, hidden ones excluded unless `where hidden` |
| `teams` | Team | my teams |
| `messages(space)` | Message | loaded messages of one space (parameter required) |
| `thread(message)` | Message | a root and its replies |
| `calls` | Call | the current call (0 or 1 today; a list so a second call fits later) |
| `call_history` | CallRecord | |
| `meetings` | Meeting | running, starting soon and calendar |
| `activity` | ActivityItem | derived "who is busy now" feed (section 9.6) |
| `notifications` | Notification | pending toasts and unread items |
| `settings` | Setting | the whitelist |
| `me`, `app` | singletons | |

### 2.4 Relations

```text
Me 1─1 Person ─1 Presence
Person 0..1─ direct Space            Space *─0..1 Team ─1 general Space
Space 1─* Message ─0..1 parent Message
Space *─* Person (members, moderators, typing)
Message 1─* Reaction *─* Person      Message 1─* File
Call 1─* Participant ─0..1 Person    Call 0..1─ Space
Meeting 0..1─ Space                  CallRecord 0..1─ Person
```

Relations are followed with `.`: `space.peer.presence.state`, `message.author.avatar`, `team.spaces`. A relation whose
target is in a scope the package lacks resolves to absent.

### 2.5 Versioning and contracts

- `webox-data/1` is the model version. The list of entities, fields, collections and functions is generated from the
  Rust definitions into `docs/data-model.json`, which the linter reads and which `host.capabilities` reports as
  `"data": 1` plus a field list.
- The action ids and state keys follow theming-engine decision 7: published from Phase 3, unstable until Phase 7.

### 2.6 Where the model lives

A `DataStore` next to `App`: per entity kind a slab (`Vec<Entity>` + `HashMap<Id, u32>`), interned strings
(`Arc<str>`), and per entity a `rev`. `App::poll` already handles every `Resp`; each arm also calls the store's
`apply_*`, which records the changed `(kind, index, field mask)` in a change list for this frame. The existing `App`
fields stay as they are during migration; the store is a second, typed view filled from the same messages, and later
the UI reads from it too.

---

## 3. Access for themes: declarative bindings

### 3.1 Data sources

A layout or view declares named sources. Each is a query, written as JSON or in a short text form that compiles to the
same tree:

```json
"data": {
  "params": { "q": "", "tab": "online", "team": null },
  "sources": {
    "online":  "people where presence.online and name ~ $q sort name limit 500",
    "dms":     { "from": "spaces", "where": "kind == 'direct' or (kind == 'group' and team == null)",
                 "sort": ["-last_activity"], "limit": 200 },
    "teamsp":  "spaces where team == $team sort -favorite, title",
    "busy":    "people where presence.in_call sort presence.state, name",
    "rail":    "teams sort name; spaces where favorite and team == null sort title",
    "call":    "calls where state != 'ended' limit 1"
  }
}
```

Grammar (the text form):

```text
query   := from [where expr] [search $param in fields] [group by path] [sort keys] [limit n] [offset n]
from    := collection | collection "(" arg ")"          messages($space)
keys    := ["-"]path ("," ["-"]path)*                     "-" = descending
expr    := expr and expr | expr or expr | not expr | "(" expr ")" | cmp
cmp     := value (== | != | < | <= | > | >= | in | ~ | contains | starts_with) value | path   (a bool path)
value   := path | "literal" | number | true | false | null | $param | @me | [value, ...] | fn(value, ...)
```

- `~` is a case- and accent-insensitive "contains" for search boxes; `search $q in name, email, presence.text` does the
  same across several fields and ranks prefix matches first.
- `$param` reads the layout's params (set by widgets such as a search input or tabs) and the theming engine's state keys
  (`$space.selected`, `$nav.page`). `@me` is the Me entity.
- Several queries separated by `;` concatenate (the rail: teams, then favourite spaces outside teams). Each part keeps
  its own sort; a `kind` field tells the template which one an item is.
- Functions in expressions: `count(path)`, `len(list)`, `any(list, expr)`, `all(list, expr)`, `now()`, `minutes(n)`,
  `hours(n)`, `days(n)`, `lower(s)`. There is **no string concatenation that yields a URL** and no function that can
  reach the network or a file.

### 3.2 Computed values and formatting

Text in templates is a format string with bindings in `{}`. Bindings may pipe through formatters:

| Formatter | Example | Result |
| --- | --- | --- |
| `count` | `{count(space.members)}` | `12` |
| `plural` | `{space.member_count | plural('member')}` | `1 member`, `3 members` (from the language pack) |
| `rel` | `{space.last_activity | rel}` | `5 min`, `yesterday`, `Mon` (Webex's own list rules) |
| `time`, `date`, `datetime` | `{meeting.start | time}` | local time in the user's format |
| `duration` | `{call.started_at | since | duration}` | `12:04` |
| `until` | `{presence.until | until}` | `until 15:00` |
| `presence` | `{person.presence | presence}` | `Available`, `In a meeting` (`presence_text`, translated) |
| `status_line` | `{person | status_line}` | custom text if set, else the presence label with `until`, else "Active 5 min ago" (the official list's rule) |
| `t` | `{t('Contacts')}` | translation from the embedded language packs (`src/assets/i18n/`) |
| `initials`, `upper`, `lower`, `trunc(n)` | | |
| `default(x)` | `{person.title | default('')}` | fallback for absent values |

Formatters are a fixed, compiled set. Time-dependent ones (`rel`, `since`, `until` with "in 5 min") register a tick:
the engine re-evaluates them once per minute (once per second only for `duration` of a running call), not per frame.

### 3.3 Templates in the widget tree

The theming engine's widget set `W` (section 4 there) gains data widgets:

```json
{ "list": "online",
  "as": "p",
  "key": "p.id",
  "row_height": "{tok:friends.row.height}",
  "group_header": { "text": "{group.key | upper} — {group.count}", "role": "caption" },
  "item": { "use": "friend_row" },
  "empty":   { "use": "empty_state", "args": { "art": "Contacts_empty_space", "text": "{t('No one is online')}" } },
  "loading": { "skeleton": { "rows": 8 } },
  "error":   { "text": "{t('Could not load')}", "action": "data.retry" } }
```

- `list` (vertical, virtualised), `grid` (wrapping, virtualised by row), `repeat` (small, not virtualised, at most 64
  items: chips, avatar stacks), `when` (a condition), `switch` (on a value), `bind` (binds a single entity: `{"bind":
  "@me", "as": "me", "child": ...}`).
- **Reusable templates** (`"templates": { "friend_row": {...} }`) take `args` and see the item variable. They are
  expanded at compile time.
- **Data widgets**: `avatar` (image + initials fallback + presence badge + typing ring, one primitive so every theme
  draws presence the same way), `presence_dot`, `badge` (count / dot / mention), `image` (only `ImageRef` or package
  assets), `rich_text` (message HTML through Webox's renderer), `time` (live relative time), `input` (writes a
  `$param`), `tabs` (writes a `$param`), `menu_button` (opens a host menu, section 5).
- Widgets inside a template refer to item fields: `{"text": "{p.name}"}`, `{"avatar": "p", "size": 32, "presence":
  true}`.

### 3.4 Live updates

- At compile time each binding records its **dependencies**: `(entity kind, field mask)` plus the sources it reads.
- A frame's change list (section 2.6) is matched against those dependencies. Only affected sources update, and only
  affected rows re-layout.
- A source keeps a sorted index list. A changed entity is re-tested against `where` and moved with a binary search
  (`O(log n)`); `group` and `limit` adjust locally. An insert or a removal is a diff on that list, which a `list`
  widget applies without rebuilding the others.
- Presence storms (hundreds of updates after reconnect) are coalesced to one pass per frame, and at most 10 passes per
  second per source.

### 3.5 Pagination and virtualisation

- `list` and `grid` lay out only the visible rows plus one screen of overscan. Row heights are fixed per template
  (`row_height`) or measured once per item `rev` and cached.
- A source never copies entities: it holds `u32` indices. `limit` caps the materialised count (default 500, maximum
  5000 per source).
- **Fetchable data** (`F` in section 1) is fetched on demand: a `list` over `messages($space)` that scrolls to the top
  triggers `Req::MessagesBefore`; a binding to `space.member_count` or `space.members` beyond the first 10 triggers
  `SpaceOp::People` once per space and session. Each fetch is rate-limited and deduplicated by the store; the binding
  shows `loading` until the answer arrives.

### 3.6 Empty, loading and error states

Every source has a state: `loading` (a fetch is pending and there is nothing yet), `ready`, `empty`, `error` (the last
fetch failed; `data.retry` asks again), `denied` (the package lacks the scope). `denied` renders as `empty` by default,
so a theme bound to an ungranted scope shows empty data and never crashes. A theme may give `denied` its own template,
for example a "Grant access" button that opens its permission page.

---

## 4. Access for native plugins

New host methods (contract 1.7, named methods; no `PluginVTable` change). They use the same model, collections and query
language as bindings.

| Method | Argument | Answer |
| --- | --- | --- |
| `data.model` | `{}` | `{"version": "webox-data/1", "entities": {...}, "collections": [...]}` (the generated description) |
| `data.get` | `{"entity": "person", "id": "...", "fields": ["name", "presence.state"]}` | `{"item": {...}, "rev": 42}` or `{"error"}` |
| `data.query` | `{"query": "<text or JSON>", "params": {...}, "fields": [...], "limit": 100, "cursor"?: "..."}` | `{"items": [...], "total": 312, "cursor"?: "...", "rev": 9}` |
| `data.subscribe` | `{"id": "online", "query": ..., "params": ..., "fields": [...]}` | the first page, like `data.query`, plus `{"sub": "online"}` |
| `data.unsubscribe` | `{"id": "online"}` | `{}` |
| `data.fetch` | `{"what": "space.members", "id": "..."}` | `{}`; the result arrives as a change |
| `ui.dispatch` | `{"action": "space.open", ...}` | `{}` or `{"refused": "<reason>"}` (theming engine §3) |

Event `data.changed` (version 1), delivered off the UI thread's hot path (queued and sent after the frame):

```json
{ "sub": "online", "rev": 10,
  "ops": [
    { "op": "insert", "at": 3, "item": { "id": "…", "name": "…", "presence": { "state": "active" } } },
    { "op": "update", "id": "…", "set": { "presence.state": "meeting", "presence.until": 1764082800000 } },
    { "op": "move",   "id": "…", "to": 0 },
    { "op": "remove", "id": "…" },
    { "op": "rekey",  "from": "email:a@b.c", "to": "8f0c…" } ] }
```

- Only the requested `fields` appear in `update.set`. A change to a field the subscriber did not ask for sends nothing.
- `rev` increases by one per event. A plugin that sees a gap (it was busy) calls `data.query` again; the host also
  sends `{"op": "reset"}` when it dropped a backlog (more than 1000 queued ops).
- At most 32 subscriptions per plugin. Events are coalesced to at most 10 per second per subscription.
- Plugins have full access (they are native code), so no scope check refuses them. Each `data.*` call is attributed to
  the plugin, and **Settings > Plugins** shows which scopes a plugin has used ("reads message content").
- The existing methods `spaces.list`, `people.search`, `messages.search` and `space.history` stay. They become thin
  wrappers over `data.query`.

---

## 5. Actions

### 5.1 Catalogue

All actions go through `App::dispatch(action, origin)` (theming engine §3). The ids follow the bus naming (`area.verb`).
"Guards" are checked for every caller; "Tier" is what a theme needs (section 5.2).

| Action | Parameters | Guards | Tier |
| --- | --- | --- | --- |
| `nav.page` | `page` | page exists | free |
| `nav.back`, `nav.forward` | | | free |
| `space.open` | `space` | space known | free |
| `space.open_direct` | `person` | person known; opens the existing 1:1 or the "new message" pane for them; never creates a space by itself | free |
| `space.new` | `people?` | opens the "new space" dialog | free |
| `panel.toggle`, `panel.set` | `panel`, `open?` | panel declared in the layout | free |
| `ui.focus_mode`, `ui.rail_wide` | `on` | | free |
| `search.open` | `query?`, `tab?` | opens the built-in search UI (the user types) | free |
| `settings.open` | `page?` | | free |
| `profile.open` | `person` | opens the built-in profile card | free |
| `menu.person`, `menu.space`, `menu.message` | `id` | opens the **built-in** context menu at the widget; the menu's items then run with built-in rules | free |
| `composer.focus`, `composer.insert` | `text?` | a writable space is open | free |
| `media.open` | `file` | opens the built-in viewer | free (needs `files` to bind the file) |
| `space.favorite` | `space`, `on` | | write.spaces |
| `space.mark_read`, `space.mark_unread` | `space` | sends a read receipt others can see | write.spaces |
| `space.notify` | `space`, `level` | | write.spaces |
| `space.hide` | `space` | | write.spaces |
| `message.react` | `message`, `name`, `on` | space writable | write.messages |
| `message.send` | `space`, `text` | space writable; plain text; at most 4 KB | write.messages |
| `message.flag` | `message`, `on` | | write.messages |
| `presence.set` | `state` (`active` / `away` / `dnd`), `for?` (duration) | the same choices as the status menu | write.presence |
| `status.set_custom` | `text`, `until?` | local only | write.presence |
| `call.start` | `person` or `space`, `video` | `locus::call_allowed`; not in a closed space; one call at a time | write.calls |
| `call.answer`, `call.decline` | `video?` | an incoming call exists | write.calls |
| `call.join` | `space` or `meeting` | a meeting is running / the meeting is mine | write.calls |
| `call.hang_up` | | a call exists | write.calls |
| `call.mute` | `on?` (toggle when absent) | a call exists; unmute refused when `unmute_allowed` is false | write.calls |
| `call.video` | `on?` | a call exists; camera present | write.calls |
| `call.share` | | opens the built-in share picker | write.calls |
| `call.hand` | `on?` | group call | write.calls |
| `audio.deafen` | `on?` | local: silences call output and ringtone (new; section 9.8) | free |
| `audio.input_mute_default` | `on` | local setting "join calls muted" (new; section 9.8) | free |
| `contact.add` | `person` | **unsupported** until the contacts service is implemented; refused with a toast | n/a |
| `data.retry`, `data.fetch` | `source` / `what`, `id` | rate-limited | free |

Every refusal shows the same toast as the built-in path and is answered to the caller (`{"refused": "..."}` for
plugins).

### 5.2 Tiers for themes

- **Free** (navigation and UI): changes only what the user sees on this computer. Always allowed.
- **Write** tiers (`write.spaces`, `write.messages`, `write.presence`, `write.calls`): change something on the Webex
  account or something other people see. A theme must declare each tier it uses; the user grants them on install
  (section 6).
- **User gesture only.** A theme action runs only from a click, a key press or a drag on the theme's own widget, in
  the frame that gesture happened. A theme cannot trigger an action from a data change, a timer or a binding. This
  makes "auto-reply" or "auto-join" impossible for themes; they are plugin behaviour.
- **Built-in confirmations stay.** For example, leaving a space, deleting a message, or "end for all" are not in the
  theme catalogue at all; a theme reaches them only through the built-in menus (`menu.*`), which apply their own
  confirmations.
- Plugins dispatch any action (they have full access), with `Origin::Plugin` for logs.

---

## 6. Privacy and permissions

### 6.1 Scopes

| Scope | Contents | Theme |
| --- | --- | --- |
| `app`, `me.basic` | app state; my name, avatar, presence, custom status | without asking |
| `people.basic`, `people.presence` | names, avatars, presence, status text of people in my spaces | without asking |
| `spaces.list` | space titles, team, picture, unread, favourite, notify, members' names, meeting running | without asking |
| `calls.state` | my current call, its participants' names, mute / video / share state | without asking |
| `meetings.list` | my meetings' titles and times | without asking |
| `settings.read` | the display-settings whitelist | without asking |
| `messages.meta` | that a message exists, its time, author and kind (enables "last activity by", typing) | without asking |
| `messages.content` | message text, HTML, previews, drafts, last-message previews, quotes, kept messages | **grant** |
| `messages.receipts` | who read what | **grant** |
| `files` | file names, sizes, thumbnails, images in messages | **grant** |
| `people.email` | other people's e-mail addresses | **grant** |
| `search` | directory search and message search results | **grant** |
| `calls.history` | call history (who called when) | **grant** |
| `calls.stats` | network statistics of the call | without asking |
| `me.devices`, `me.features` | device names, feature toggles | **grant** |
| `notifications` | pending notifications (bodies need `messages.content` too) | without asking |

"Without asking" scopes still have to be **declared** in the manifest. Webox shows them on the install screen as
information ("This theme shows: people's presence, your spaces, your calls"). Undeclared scopes are never served, even
if they would be free.

### 6.2 Manifest

```json
{
  "schema": "webox-ui/1", "kind": "theme",
  "id": "dev.example.night-chat", "name": "Night Chat", "version": "1.0.0", "minWebox": "0.5.0",
  "requires": { "data": "webox-data/1", "components": { "message_list": 1, "space_list": 1 } },
  "data_scopes": ["me.basic", "people.basic", "people.presence", "spaces.list", "calls.state", "meetings.list",
                  "messages.content"],
  "actions": ["write.calls", "write.presence"],
  "remote_images": [],
  "provides": { "themes": [...], "layouts": [...], "templates": [...] }
}
```

### 6.3 How the user sees and grants it

- **Install** (from a repository or a local folder): a sheet lists what the theme shows, the sensitive scopes as
  switches (off by default), the write tiers as switches (off by default), and the image domains (section 7). "Install"
  applies the choices.
- **Settings > Themes > <theme>**: the same switches at any time, plus "Last used" per sensitive scope.
- **Not granted** means empty data (`denied` → `empty`) or a disabled button with the tooltip "Not allowed for this
  theme". Nothing crashes, and the theme does not have to handle it specially.
- **Updates** that add a scope, a tier or an image domain wait for approval: the old version stays active and the
  Themes tab shows "Update needs approval".
- **Plugins** keep the single warning. Their page lists the scopes they actually used (from the `data.*` log), so the
  user can see what a plugin reads.

### 6.4 What never happens

- Webex data never leaves the machine through this API. Themes have no network access, no file access and no code;
  their only outputs are pixels and actions on the bus.
- Image fields are handles, not URLs; tokens and authenticated URLs are never exposed.
- Settings that hold secrets (`token`, `refresh_token`, device ids, `idbroker_base`, paths, the e-mail) are not in the
  whitelist.
- A theme cannot read another theme's or a plugin's state, and cannot read what the user types in the composer.

---

## 7. Images in themes

A remote image URL is a network request. A theme could exfiltrate bound data through it
(`https://x/p.png?msg={message.text}`) or track the user (IP address, usage times). The rules:

1. **Bundled assets first.** Images (PNG, SVG, WebP, GIF), fonts and icons ship inside the theme package
   (`images/`, `fonts/`, `icons/`, with the size caps of theming engine §1 and §5). That covers nearly every theme.
2. **Webex images come from Webox.** Avatars, covers, space and team pictures, file thumbnails, link-preview images and
   emoji are bound as data (`person.avatar`, `space.avatar`, `file.thumb`, ...). Webox loads them through its existing
   authenticated caches (`src/assets/photos/`). The theme never sees a URL or a token.
3. **Remote images only as static URLs** listed in the manifest (`"remote_images": ["https://cdn.example.com/bg.webp"]`).
   A URL can never contain a binding or an expression: the loader and the linter reject `{`, `$` and any template
   syntax in an image URL, an `image` widget accepts only a package asset name, a manifest URL by its exact text, or an
   `ImageRef`, and the expression language has no string concatenation that could build a URL.
4. **Webox fetches them, not the theme**: once at install or update, HTTPS only, no cookies, no referrer, a fixed
   generic user agent, at most 10 MB per image and 50 MB per theme, image types only. The bytes are decoded and
   re-encoded (never handed through raw), then cached locally. They are not fetched again while the app runs, so
   usage times cannot be tracked.
5. **The install screen lists the domains** ("This theme loads images from cdn.example.com") and the user can decline.
   The theme then falls back to its bundled assets for that image (`"fallback": "images/bg.png"`) or to nothing, and
   does not break.
6. **Animated images** follow the existing GIF rules (`src/assets/photos/`): paused when the window is unfocused
   (`gifs_pause_unfocused`), and the frame and texture budgets (`MAX_FRAMES`, `MAX_FRAME_SIDE`).

Fonts are bundled only. There are no remote fonts.

---

## 8. Performance

The standing rule: Webox uses less CPU and RAM than the official Webex app. The data layer must not change that.

- **Compiled once.** A source query compiles at package load into a small typed tree: field paths become
  `(kind, field index)` pairs, `$params` become slots, string literals are interned. No parsing or string lookups
  happen afterwards.
- **Incremental, never per frame.** Evaluation runs only on changes (section 3.4): store changes, a param change from
  an input, or a minute tick for relative times. An idle window does no data work, and the theming engine's "no
  repaint when idle" check (`WEBOX_PROBE=1`) applies.
- **Bounded cost per change.** Re-testing one entity against a source is `O(predicate)`, re-sorting it `O(log n)`.
  A full re-evaluation (param change, such as typing in a search box) is `O(n log n)` over at most the source limit, and
  typing is debounced to 100 ms.
- **Virtualised rendering.** Only visible rows are laid out and painted. A list of 5000 people costs the same per frame
  as a list of 30.
- **Memory.** Sources hold `u32` indices into the store, not copies. The store uses interned `Arc<str>` shared with the
  existing `App` data where possible. Budget per theme: 32 sources, 5000 rows per source, 256 template nodes per item,
  and about 1 MB of index memory.
- **Fetching on demand stays polite.** `F` data is fetched only when a visible widget needs it, once per session and
  entity, rate-limited (at most 2 requests per second from bindings), and never polled.
- **Plugins**: subscription diffs carry only the asked fields and are coalesced, so a busy plugin cannot slow the UI.
- **Measured**: the parity harness gains a "Night Chat" scene, and the CPU and RAM comparison against the official app
  runs with it active.

---

## 9. The first theme: "Night Chat" (server-bar layout)

The user's reference is a saved page of a popular gaming chat app on its friends page (dark, "darker" variant, compact
density, 16 px base font). Only its structure and sizes were read. No names, content, logos, icons, artwork, sounds or
its proprietary font are used: the theme uses Webox's Momentum icons and a freely licensed font (Inter or Noto Sans,
OFL), and its own name.

### 9.1 Regions of the reference and their mapping

| Region (reference) | Size (reference, px) | Webox component / view | Data | Actions |
| --- | --- | --- | --- | --- |
| Title bar (window controls) | ~32 high | `header` variant `compact` (window chrome stays global) | `app.connected` | global search pill → `search.open` |
| Server bar: home button, separator, server icons and folders (colour-tinted), add button | 72 wide; icons 40 × 40; folder previews | `rail` variant `source: teams_and_spaces`, `item: rounded` | `teams`, favourite `spaces` outside teams; `unread`, `mentioned` | home → `nav.page messaging` + clear team; team → set `$team`; space → `space.open`; "+" → `space.new` |
| Channel sidebar (resizable) | 383 wide in the reference (resized; default ~240), resize handle | `space_list` with a theme template, in a `Split` | `dms` source | row → `space.open`; row close "×" → `space.hide` (write.spaces); right-click → `menu.space` |
| Sidebar top: search button, nav rows (friends etc.), "Direct messages" header with "+" | rows ~42 high | view `nav_block` | `app.page` | "Find or start a conversation" → `search.open`; Friends → `nav.page contacts`; "+" → `space.new` |
| DM rows | ~42 high, avatar 32, presence dot, name, status line or member count, close button | template `dm_row` | `space.peer.presence`, `status_line`, `member_count` | as above |
| Self panel (bottom of sidebar) | 58 high (`--custom-app-panels-height`), avatar 32 | view `self_panel` | `@me` | mic → `call.mute` / `audio.input_mute_default`; headset → `audio.deafen`; gear → `settings.open`; avatar → status menu (`menu.status`, built-in) |
| Centre header: title "Friends", tabs, primary "Add friend" button | ~48 high | view `contacts_header` | `$tab` | tabs write `$tab`; "Add" → `space.new` (mapped, 9.7) |
| Search field | ~36 high | `input` → `$q` | | |
| Section header "Online — N" | ~32 | text | `count(online)` | |
| People rows | 61 high (16 rows in the reference), avatar 32, name, status line, two round action buttons (message, more) | template `friend_row` in a virtualised `list` | `contacts` / `online` / `pending` sources | message → `space.open_direct`; more → `menu.person`; row click → `profile.open` |
| "Active now" column | ~360 wide | view `active_now` | `busy` and running meetings, my call | card click → `profile.open` or `space.open`; "Join" → `call.join` (write.calls) |
| Active-now cards: avatar 32–40, name, activity line, sub-card with the activity and an avatar stack of party members | cards ~16 padding, radius 8 | template `activity_card` | `activity` collection (9.6) | |

### 9.2 Layout tree

```json
{
  "schema": "webox-ui-layout/1", "id": "night-chat", "name": "Night Chat",
  "panels": { "active_now": { "default_open": true } },
  "data": {
    "params": { "q": "", "tab": "online", "team": null },
    "sources": {
      "rail_teams":  "teams sort name",
      "rail_spaces": "spaces where favorite and team == null sort title",
      "dms":         "spaces where (kind == 'direct' or team == null) and not hidden and (team == $team or $team == null) sort -last_activity limit 300",
      "team_spaces": "spaces where team == $team sort -favorite, title",
      "online":      "contacts where presence.online search $q in name, presence.text sort name",
      "all":         "contacts search $q in name, presence.text sort name",
      "pending":     "spaces where kind == 'direct' and unread sort -last_activity; call_history where missed and at > now() - days(7) sort -at",
      "busy":        "activity sort -since limit 20",
      "call":        "calls where state != 'ended' limit 1"
    }
  },
  "surfaces": { "main": {
    "switch": "window.width_class",
    "cases": { "narrow": { "slot": "space_list", "variants": { "row": "one_line" } } },
    "default": {
      "dock": "left", "id": "server-bar", "size": { "pref": "{tok:night.rail.width}" },
      "child": { "view": "server_bar" },
      "rest": {
        "split": "row", "id": "main-split", "items": [
          { "size": { "min": 220, "pref": 300, "max": 420, "resizable": true },
            "node": { "stack": "column", "id": "sidebar", "children": [
              { "view": "nav_block" },
              { "switch": "$team", "cases": { "null": { "view": "dm_list", "grow": true } },
                "default": { "slot": "space_list", "grow": true, "variants": { "row": "one_line", "sections": "flat", "preview": false } } },
              { "view": "self_panel" } ] } },
          { "size": { "min": 480 }, "node": { "switch": "nav.page", "cases": {
              "contacts":  { "stack": "column", "id": "friends", "children": [
                { "view": "contacts_header" }, { "view": "contacts_list", "grow": true } ] },
              "messaging": { "stack": "column", "id": "chat", "children": [
                { "slot": "chat_header", "variants": { "size": "bar", "actions": "icons" } },
                { "slot": "message_list", "grow": true, "variants": { "style": "compact", "timestamp": "hover", "avatar": "circle" } },
                { "slot": "composer", "variants": { "toolbar": "inline", "shape": "card" } } ] },
              "settings": { "split": "row", "items": [
                { "size": { "pref": 220 }, "node": { "slot": "settings_nav" } }, { "node": { "slot": "settings_page" } } ] },
              "*": { "slot": "page" } } } },
          { "size": { "min": 300, "pref": "{tok:night.active.width}", "max": 420, "collapse_below": 1100 },
            "when": { "all": [{ "key": "nav.page", "eq": "contacts" }, { "key": "panel.active_now.open" }] },
            "node": { "view": "active_now" } } ] } } } },
  "reach": { "settings": "self_panel.gear", "space_list": "server_bar.home" }
}
```

### 9.3 Views and templates

Server bar:

```json
"server_bar": { "column": { "gap": "{tok:night.rail.gap}", "align": "center", "children": [
  { "button": { "icon": "chat-filled", "style": "rail_home", "action": "nav.page:messaging",
                "set": { "$team": null }, "selected": "{$team == null}", "tooltip": "{t('Direct messages')}" },
    "badge": { "count": "{app.unread_total}", "tone": "danger", "hide_zero": true } },
  { "separator": { "length": 32 } },
  { "list": "rail_teams;rail_spaces", "as": "it", "key": "it.id", "row_height": 48, "item": { "use": "rail_item" } },
  { "button": { "icon": "plus-regular", "style": "rail_add", "action": "space.new", "tooltip": "{t('Create a space')}" } } ] } }

"rail_item": { "row": { "children": [
  { "pill": { "state": "{it.id == $team or it.id == $space.selected ? 'selected' : it.unread ? 'unread' : 'none'}" } },
  { "avatar": "it", "size": "{tok:night.rail.icon}", "shape": "{it.id == $team ? 'rounded' : 'circle'}",
    "color": "{it.color}", "badge": { "mention": "{it.mentioned}" },
    "action": "{it.kind == 'team' ? 'set:$team' : 'space.open'}", "tooltip": "{it.name}",
    "menu": "{it.kind == 'team' ? 'menu.team' : 'menu.space'}" } ] } }
```

DM list and row:

```json
"dm_list": { "column": { "children": [
  { "row": { "children": [ { "text": "{t('Direct messages')}", "role": "caption" }, { "spacer": {} },
             { "button": { "icon": "plus-regular", "style": "ghost", "action": "space.new", "tooltip": "{t('New message')}" } } ] } },
  { "list": "dms", "as": "s", "key": "s.id", "row_height": "{tok:night.dm.row}",
    "item": { "use": "dm_row" },
    "empty": { "text": "{t('No conversations yet')}", "role": "secondary" } } ] } }

"dm_row": { "list_row": {
  "selected": "{s.id == $space.selected}", "action": "space.open", "menu": "menu.space",
  "leading": { "avatar": "{s.kind == 'direct' ? s.peer : s}", "size": "{tok:night.dm.avatar}", "presence": "{s.kind == 'direct'}" },
  "title": { "text": "{s.title}", "weight": "{s.unread ? 'semibold' : 'regular'}" },
  "subtitle": { "text": "{s.kind == 'direct' ? (s.peer | status_line) : (s.member_count | plural('member'))}" },
  "trailing": { "row": { "children": [
    { "badge": { "mention": "{s.mentioned}" } },
    { "button": { "icon": "cancel-regular", "style": "ghost_hover", "action": "space.hide", "tooltip": "{t('Hide')}" } } ] } } } }
```

Contacts (centre):

```json
"contacts_header": { "row": { "gap": 16, "children": [
  { "icon": { "name": "people-regular" } }, { "text": "{t('Contacts')}", "role": "title" }, { "separator": { "vertical": true } },
  { "tabs": "$tab", "items": [
    { "id": "online",  "label": "{t('Online')}" },
    { "id": "all",     "label": "{t('All')}" },
    { "id": "pending", "label": "{t('Pending')}", "badge": "{count(pending)}" } ] },
  { "button": { "label": "{t('New message')}", "style": "filled_small", "action": "space.new" } },
  { "spacer": {} },
  { "button": { "icon": "panel-right-regular", "style": "ghost", "action": "panel.toggle:active_now" } } ] } }

"contacts_list": { "column": { "children": [
  { "input": "$q", "placeholder": "{t('Search')}", "icon": "search-regular" },
  { "switch": "$tab", "cases": {
    "online":  { "use": "people_section", "args": { "src": "online", "label": "Online" } },
    "all":     { "use": "people_section", "args": { "src": "all", "label": "All contacts" } },
    "pending": { "use": "pending_section" } } } ] } }

"people_section": { "column": { "children": [
  { "text": "{t(label) | upper} — {count(src)}", "role": "caption" },
  { "list": "{src}", "as": "p", "key": "p.id", "row_height": "{tok:night.people.row}", "divider": true,
    "item": { "use": "friend_row" },
    "empty": { "use": "empty_state", "args": { "art": "Contacts_empty_space", "text": "{t('Nobody here yet')}" } },
    "loading": { "skeleton": { "rows": 8 } } } ] } }

"friend_row": { "list_row": {
  "action": "profile.open", "menu": "menu.person",
  "leading": { "avatar": "p", "size": "{tok:night.people.avatar}", "presence": true },
  "title": { "text": "{p.name}", "weight": "semibold" },
  "subtitle": { "text": "{p | status_line}" },
  "trailing": { "row": { "gap": 10, "children": [
    { "button": { "icon": "chat-regular", "style": "round_plated", "action": "space.open_direct", "tooltip": "{t('Message')}" } },
    { "button": { "icon": "more-regular", "style": "round_plated", "action": "menu.person", "tooltip": "{t('More')}" } } ] } } } }
```

Active now:

```json
"active_now": { "column": { "gap": 16, "children": [
  { "text": "{t('Active now')}", "role": "title" },
  { "list": "busy", "as": "a", "key": "a.id", "item": { "use": "activity_card" },
    "empty": { "column": { "align": "center", "children": [
      { "text": "{t(\"It's quiet for now...\")}", "role": "body_semibold" },
      { "text": "{t('When someone is in a call or meeting, it shows here.')}", "role": "secondary" } ] } } } ] } }

"activity_card": { "card": { "style": "active_card", "action": "{a.space ? 'space.open' : 'profile.open'}", "child": { "column": { "children": [
  { "row": { "children": [
    { "avatar": "a.person", "size": 32, "presence": true },
    { "column": { "children": [
      { "text": "{a.person.name}", "weight": "semibold" },
      { "text": "{a.label} · {a.since | rel}", "role": "secondary" } ] } } ] } },
  { "when": "a.space", "child": { "card": { "style": "active_subcard", "child": { "row": { "children": [
    { "avatar": "a.space", "size": 40 },
    { "column": { "children": [ { "text": "{a.space.title}" }, { "text": "{a.participants | len | plural('participant')}", "role": "secondary" } ] } },
    { "spacer": {} },
    { "avatar_stack": "a.participants", "size": 24, "max": 4 },
    { "when": "not a.mine", "child": { "button": { "label": "{t('Join')}", "style": "filled_small", "action": "call.join" } } } ] } } } } } ] } } } }
```

Self panel:

```json
"self_panel": { "bind": "@me", "as": "me", "child": { "row": { "style": "self_panel", "children": [
  { "avatar": "me.person", "size": 32, "presence": true, "action": "menu.status", "tooltip": "{t('Set a status')}" },
  { "column": { "grow": true, "children": [
    { "text": "{me.person.name}", "weight": "semibold", "max_lines": 1 },
    { "text": "{me.custom_status.text | default(me.person.presence | presence)}", "role": "caption", "max_lines": 1 } ] } },
  { "toggle_button": { "icon_on": "microphone-muted-regular", "icon_off": "microphone-regular",
      "on": "{call.me.muted | default(settings.join_muted)}",
      "action": "{call ? 'call.mute' : 'audio.input_mute_default'}", "tooltip": "{t('Mute')}" } },
  { "toggle_button": { "icon_on": "headset-muted-regular", "icon_off": "headset-regular", "on": "{settings.deafened}",
      "action": "audio.deafen", "tooltip": "{t('Silence audio')}" } },
  { "button": { "icon": "settings-regular", "style": "ghost", "action": "settings.open", "tooltip": "{t('Settings')}" } } ] } } }
```

### 9.4 Theme tokens

Estimated from the reference's inline sizes (icons 40 × 40, avatars 32 × 32 and 48 × 48, people rows 61 high, the
sidebar width variable 383 and the panels height 58) and its "darker" dark palette. Colours are generic dark greys
chosen for the theme, with its own accent (not the reference's brand colour).

| Token | Value | Notes |
| --- | --- | --- |
| `night.rail.width` | 72 | server bar |
| `night.rail.icon` | 40 | team / space icons (rounded 12 when selected, circle otherwise) |
| `night.rail.gap` | 8 | |
| `night.rail.pill` | 4 × 8 (unread), 4 × 20 (hover), 4 × 40 (selected) | left indicator, radius 0 4 4 0 |
| `night.sidebar.width` | 300 (resizable 220–420) | reference resized to 383 |
| `night.dm.row` | 42 | DM rows, radius 8, 8 side margin |
| `night.dm.avatar` | 32 | presence dot 10 with a 3 px cut-out ring |
| `night.self.height` | 58 | self panel; radius 8, 8 margin; avatar 32 |
| `night.header.height` | 48 | centre header |
| `night.search.height` | 36 | radius 8 |
| `night.people.row` | 61 | 60 + 1 divider |
| `night.people.avatar` | 32 | |
| `night.people.action` | 36 | round plated buttons |
| `night.active.width` | 360 | active-now column |
| `night.card.radius` | 8 | active-now cards; padding 16 |
| `night.badge.height` | 16 | radius 8, min width 16, 12 px bold text |
| `density` | `compact` | the theme's base density |
| `fonts.scale.body` | 15 | reference base 16 px ≈ 15 pt at Webox's scale |
| `fonts.families.body` | `Inter` (OFL), `@system` fallback | no proprietary font |
| `role.win` (server bar) | `#121214` | |
| `role.card` (sidebar) | `#18181B` | |
| `role.content` (centre) | `#1C1C20` | |
| `role.surface_high` (cards, menus) | `#242429` | |
| `role.text1` | `#E6E7EA` | |
| `role.text2` | `#9A9CA5` | |
| `role.accent` | `#6E7BF2` | the theme's own indigo |
| `presence.online` | `#45A366` | |
| `presence.idle` | `#E0A83A` | away / inactive |
| `presence.dnd` | `#E5484D` | also `badge.danger` |
| `presence.call` | `#45A366` with a headset glyph | call / meeting / presenting |
| `list.row.hover` / `selected` | `#FFFFFF0F` / `#FFFFFF1A` | |
| `divider` | `#FFFFFF14` | |

Variants used: `message_list.style = compact`, `chat_header.size = bar`, `composer.toolbar = inline`,
`composer.shape = card`, `rail.source = teams_and_spaces`.

### 9.5 Panel-by-panel data coverage

| Panel | Covered | Gaps |
| --- | --- | --- |
| Server bar | teams (name, colour, initials), favourite spaces, unread dot, selection pill, total unread | a per-space **mention** badge after restart (M); unread **counts** (M; the dot is used) |
| DM list | 1:1 and group spaces, avatars, presence, status line, member count (F), hide | the last-message preview needs `messages.content` (granted in the manifest) |
| Contacts | online / all, search, presence, status line, message and more buttons | the real Webex contact list (M; Webox's contacts = people of my spaces), "add contact" (M) |
| Pending tab | **mapped**: unread 1:1 conversations plus missed calls of the last 7 days | Webex has no friend requests; the literal meaning is unsupported. Missed calls need `calls.history` (grant) |
| Active now | people in a call, meeting or presenting (presence), running meetings in my spaces with participants (F), my own call with its roster | **what** and **with whom** another person is in a call is not offered by Webex (unsupported); the card shows the state ("In a meeting", "Presenting") and the end time when known |
| Self panel | name, avatar, presence, custom status, status menu, mute in a call, settings | "join calls muted" and "deafen" are new local settings (9.8) |

### 9.6 The `activity` collection

A derived collection for "Active now":

```text
ActivityItem
  id: string
  person?: → Person            a busy person, or the first participant of a space meeting
  space?: → Space              a meeting running in one of my spaces
  kind: "call" | "meeting" | "presenting" | "space_meeting" | "my_call"
  label: string                translated ("In a call", "In a meeting", "Presenting", "Meeting in <space>")
  since: ms                    when the state started (presence order time or locus start)
  until?: ms
  participants: [→ Person]     only for space meetings (F, needs the locus GET) and my own call
  mine: bool                   I am in it
```

Built from `Presence.state in [call, meeting, presenting]` for people I share a space with, `Space.meeting.active`, and
`calls`. A person who is in a running space meeting appears once, under the space.

### 9.7 Mappings for things Webex does not have

- **"Add friend"** → "New message" (`space.new`): the closest real action, opening Webox's own new-message pane with the
  directory search. A real `contact.add` waits for the contacts service.
- **"Pending"** → unread 1:1 conversations and recent missed calls (9.5).
- **Server folders** → teams (a team groups spaces like a folder; the team colour tints the icon). Webex has no
  user-made folders.
- **Voice channels / "in a voice channel with"** → space meetings, which are visible with participants.
- **Activity "playing a game"** → custom status text and the presence end time.

### 9.8 What the engine and the data layer must build for this theme

Engine (theming-engine phases):

1. `rail.source = teams_and_spaces` with team selection (`$team`) and the pill indicator (P6).
2. Data widgets: `list` (virtualised), `avatar` with presence and badge, `avatar_stack`, `badge` (count, dot,
   mention), `tabs`, `input`, `toggle_button`, `card` with an action, `list_row` with leading / trailing, `pill` (P8 + D2).
3. Layout params (`$team`, `$tab`, `$q`) written by widgets and read by `switch` and sources (D2).
4. The `contacts` page as a layoutable page (today `src/ui/contacts_page.rs` draws it as a whole), so the layout can
   replace the centre with views (P5-like componentisation of one page).

Data layer:

1. The store and the collections `people`, `contacts`, `spaces`, `teams`, `calls`, `call_history`, `meetings`,
   `activity` (D0, D2).
2. `Space.mentioned` from live and loaded messages, and the member count via `SpaceOp::People` on demand (D2).
3. The locus GET of a space meeting I am not in, for the participants of "Active now" (D6).
4. Two new local settings and actions: `audio.deafen` (silence call output and ringtone) and
   `audio.input_mute_default` (join calls muted) (D1).
5. `menu.person`, `menu.space`, `menu.team`, `menu.status` as dispatchable actions that open the existing built-in menus
   at a widget's rect (D1).

---

## 10. Phases

How this fits the theming engine's plan (`docs/theming-engine.md` §7). "D" phases are this document's; effort in
agent-days.

| Phase | Content | With | Effort |
| --- | --- | --- | --- |
| **D0** store | `DataStore`, entity structs, `apply_*` in `App::poll`, change list per frame, stable ids and rekey, `docs/data-model.json` generation | after P1, parallel to P2 | 4–5 d |
| **D1** actions | the action catalogue of section 5 on the P3 bus, tiers and `Origin::Theme`, gesture-only rule, `menu.*` actions, `audio.deafen`, `audio.input_mute_default`, refusal toasts | P3 | 3–4 d (on top of P3) |
| **D2** bindings | query parser and compiler, incremental sources, formatters, ticks, `list` / `grid` / `repeat` / `bind`, data widgets, params, empty / loading / denied states, the `activity` collection, `Space.mentioned`, on-demand fetches | P8 | 10–12 d |
| **D3** plugin access | `data.model`, `data.get`, `data.query`, `data.subscribe` / `unsubscribe`, `data.changed` diffs, coalescing, usage log; old methods as wrappers | after D0 (independent of the renderer) | 3–4 d |
| **D4** two kinds | Settings > Themes and Plugins tabs, `kind: "theme"` index entries, install sheet with scopes, tiers and domains, grants storage, update approval rules, strict theme lint | P7 | 4–5 d |
| **D5** images | `remote_images` fetcher (install time, limits, re-encode, cache), URL lint, fallbacks | P7 | 2–3 d |
| **D6** new data | per item, product work: space meeting participants (F), member counts (F), mention flag after restart (M, trace first), Webex contact list and add contact (M), job titles (M), pinned messages (M), server-side search (M) | after D2, as needed | 1–3 d each |
| **D7** Night Chat | the theme package in `plugins/` (data only), its parity shots and behaviour-suite run, the CPU / RAM comparison | after P6, P8, D2, D4 | 3–4 d |

Total for D0–D5 and D7: about 29–37 agent-days on top of the theming engine, plus D6 items as needed.

**Quick wins** that can ship before layouts exist:

1. **D0 + D3**: native plugins get `data.query` / `data.subscribe` over everything Webox already has (57 available
   items). This needs no renderer and gives plugin authors the full model at once.
2. **D1's `ui.dispatch`** for plugins with the navigation and call actions, which theming-engine decision 7 already
   wants early.
3. **The `activity` collection and `Space.mentioned`** also help the built-in UI (an "active now" hint in the contacts
   page, a mention badge in the space list).

---

## Implementation notes (D0 + D3)

What `src/extras/data/` adds to or decides beyond the design above. All additions follow the additive-only rule.

- **Store.** Per entity kind a table of rows (one value per model field, in the model's order) with a revision per row.
  `App::poll` marks the kinds each `Resp` touches (`src/app/data_sync.rs`); a cheap fingerprint of the window's own state
  (page, selection, unread / favourite / hidden flags, drafts, flags, settings, call controls, toasts, plugins) catches
  what the UI changes without a `Resp`. Only marked kinds are rebuilt and compared field by field; the change list holds
  inserted, removed and updated entities with a field mask, and rekeys. A frame where nothing changed does no store
  work, and nothing at all happens while no plugin uses the data API (the store is empty then).
- **Additional fields:** `person.contact`, `person.directory`, `person.me`; `message.author_name`, `message.kept`;
  `call.peer_name`, `call.remote_share_name`, `call.remote_video`; `meeting.source`; `app.status`, `app.team`;
  `plugin.settings`. `message.author` is optional (system lines have none). `space.archived`, `space.unread_count`,
  `person.title` / `department` / `phone` / `external`, `app.width_class` and `app.focus_mode` are **reserved**: in the
  model, always absent for now.
- **Additional collections and entities:** `presence` (all presence entries), `reminders` (entity `reminder`: message
  reminders and scheduled messages, item 1.5 "Reminders, scheduled messages"), `plugins` (entity `plugin`: item 1.8
  "Plugin list, enabled state, plugin settings"; password settings are left out).
- **Read when asked (volatile):** `me.cameras`, `me.audio.inputs` / `outputs` (device enumeration blocks, so never on the
  UI thread, cached 10 s), `me.features` and `call.stats` (present while the call statistics view runs). They can be
  selected, not filtered or sorted, and `data.changed` does not follow them.
- **Not yet followed live:** typing indicators that expire without a "stopped typing" event (they update with the next
  change). `notifications` lists the toasts on screen, not unread spaces (use `spaces where unread`).
- **Ids:** a person's id is `Person.id` as Webox has it (the participant id the space list uses), or
  `email:<address>` for message authors and reactors no list knows yet; when a list brings the id, subscriptions get a
  `rekey`. A message without an id (offline samples) gets `<space>#<index>`. Meetings of the loci poll use
  `loci:<join link>`; the current call `call:<e-mail>:<start>`.
- **Plugin permissions.** As section 4 says, no scope refuses a plugin; each read is recorded by scope per plugin
  (`Hub::scopes_used`). Showing it on Settings > Plugins is a follow-up in `src/ui/`.
- **Contract 1.7** marks plugins that need the `data.*` names; there is no table change.
