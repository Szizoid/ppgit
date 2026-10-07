# ppgit

*[English](#english) | [Русский](#русский)*

## English

`ppgit` (private-public git) — a CLI wrapper over `git` for projects that have
a public part (open-source code, README, documentation) and a private part
(notes, `CLAUDE.md` and anything that shouldn't be visible to outsiders).

Idea: one working directory, but two repositories and two GitHub remotes — a
regular public one (`repo-name`) and a private one that contains the public
part plus the private files (`pp-repo-name`). `ppgit` takes care of the
routine of keeping them in sync.

### Status

The project is at an early stage. `ppgit` is a transparent passthrough
wrapper over `git`, plus a handful of its own commands:

- `ppgit <command>` forwards everything as-is to `git <command>` (arguments
  are passed through via `args_os`, so non-UTF-8 paths aren't mangled), and
  the real exit code is propagated back, including the case where `git` gets
  killed by a signal (on Unix).
- `ppgit -V`/`--version` and `ppgit -h`/`--help`/a bare `ppgit` print ppgit's
  own version/short help — everything else still goes straight to `git`.
- `ppgit init` sets up both halves of the public/private split:
  - locally — creates `.git` (the public, completely standard repository) if
    it's missing, a `.ppgitignore` template (left alone if it already
    exists), and the private `.ppgit` git-dir. In an existing project the
    private half starts as a bare *clone* of the public history rather
    than empty, so the two share an ancestry instead of the first private
    commit re-creating the whole project from nothing;
  - on GitHub (via `gh`, after checking it's installed and logged in) —
    creates the public (`<dir-name>`) and private (`pp-<dir-name>`)
    repositories if they don't already exist, points `origin` in each
    git-dir at the right one (over SSH or HTTPS, whichever `gh` is
    configured for), and turns on `push.autoSetupRemote` so the first push
    of a branch needs no `--set-upstream`.

  Every step is safe to run more than once — nothing gets recreated or
  duplicated on a second `ppgit init`.
- `ppgit clone <repository> [<directory>]` is the other way in: it sets an
  existing project up on a second machine, where `ppgit init` has nothing to
  work from. The repository can be named any way `gh` accepts one — `name`,
  `owner/name` or a URL — and can be *either* half: ppgit checks its
  visibility, works out the counterpart (`name` ↔ `pp-name`), and refuses
  the whole thing if that counterpart isn't there, rather than half-building
  a project out of a repository that never had a private side.

  The public half is cloned normally, the private one as a bare `.ppgit`
  git-dir over the same working tree; the working tree then comes from the
  private HEAD, since it's the superset. A bare clone needs two things put
  back that `git clone --bare` doesn't set up — a fetch refspec (without it
  no branch has an upstream, so `pull` is left guessing and `status` can
  never report ahead/behind) and an index (without it every tracked file
  reads as deleted) — and `ppgit clone` does both, so the result is a
  project you can `pull` and `commit` in immediately. If the two halves come
  down on different default branches, that's reported: they share a working
  tree, so ppgit needs them in step. And if the pair was already out of
  step *on the remotes* — public files the private repository doesn't
  hold, or holds at older content — the clone says so right away instead
  of leaving that for the first `doctor` to find.
- Commands are routed to one repository or both. `add`, `status`,
  `rm`, `mv`, `restore`, `push` and `fetch` run against both by
  default; everything else describes history, which the two are entitled to
  disagree about, and goes to the public one — so a bare `ppgit log` shows
  what a bare `git log` would. A leading `--public`, `--private` or
  `--both` overrides that. In dual runs the private (superset) repository
  goes first and alone decides the exit code: the public one deliberately
  can't see every file, so it having nothing to do is an ordinary outcome,
  not a failure. `commit` is a step further than "defaults to both": see
  below, it refuses to be scoped at all.
- `ppgit pull` is orchestrated rather than mirrored, because two plain
  pulls fight over the shared working tree: whichever half pulled first
  would update it, and the other half's merge then refuses to
  "overwrite local changes" (git judges those against its index, so the
  file already holding the merge result doesn't help). Instead the
  private (superset) half pulls for real — its merge is what the tree
  should hold — and the public half is then caught up without touching
  the tree: a fetch, and a fast-forward of ref and index only, guarded
  by an ancestor check so nothing is ever discarded. A public half that
  has genuinely diverged from its remote is reported and left exactly
  as it was. An explicit `--public`/`--private` pull stays passthrough.
- `stash` and `clean` are routed specially, because both concern the
  whole shared working tree, which only the private (superset) half sees
  in full. `stash` goes to the private half by default — its stash is
  the complete one, where a public stash would quietly leave the private
  files' changes in place (`--public` still narrows it deliberately;
  `--both` is refused, since whichever half stashed first would sweep
  away what the second was about to save). `stash --all` is refused
  outright: it stashes ignored files, the private git-dir is itself
  ignored, and git would stash the repository into itself and delete it
  — verified, unfortunately. `clean` runs on the private half only,
  whose untracked set is the true one (to the public half every private
  file is merely "ignored", so a public `clean -x` would delete them
  all), and ppgit shields the private git-dir itself with an injected
  `-e`. Outside a ppgit project both pass through to git untouched.
- Branch commands (`branch`, `checkout`, `switch`, `merge`) always run
  against both and refuse `--public`/`--private`. The two repositories
  share one working tree, so a branch existing in only one of them — or
  the two sitting on different branches — would send the next commit
  somewhere the other can't follow. (`checkout -- <path>` is a file
  operation rather than a branch one, and takes scope flags as usual.)
  Switching to a branch whose tracked files actually differ from the
  current one is more than "run twice": a naive double checkout hits the
  same "would overwrite local changes" wall `pull` and `cherry-pick`
  document below, since the private checkout rewrites the shared working
  tree before the public one gets a turn. `ppgit checkout <branch>`
  (a single bare branch name both halves already have) avoids it instead
  of hitting it: only the private half runs a real checkout; the public
  half is brought in line with a ref switch and an index sync that never
  touch the working tree, rather than a second real checkout. Anything
  this doesn't specifically handle (`-b`, several arguments, a raw
  commit-ish) still passes through the plain double invocation.
- `mv` runs on both by default, but not as two `git mv`s: `git mv` moves
  the file on disk too, so the second one would find its source already
  gone. The real `git mv` runs on the private half only; the public half
  then gets the same rename in its index alone (`update-index`), for the
  files it actually tracks — moving a directory with private files inside
  stages only its public ones publicly. What's mirrored is checked against
  what the private `git mv` really did, so `-n` and `-k` behave as usual.
- `ppgit commit` opens the editor once, not once per repository: the
  private commit is made first, interactively, and its message is reused
  verbatim for the public one. With an explicit `-m` (or `-F`, `--fixup`,
  ...) there's nothing to share and both simply get the same arguments. A
  half with nothing staged is skipped rather than treated as a failure —
  including the reverse of the usual case: if only the *public* index has
  something staged (after `privatize`, say), a plain `pp commit` commits
  that alone, with nothing to reuse from a private side that had nothing
  to do. `commit` refuses `--public`/`--private` outright, unlike
  everything else on this page — a commit scoped to one half happens in a
  separate invocation from its counterpart, if it ever gets one, so
  there's no single moment where the two could be tied together as one
  logical change. Nothing is lost: the cases a narrow commit was ever
  used for (a change touching only one half) already commit correctly
  through the plain, unscoped form described above.
- `pp commit --private <text>` adds `<text>` to the private commit's
  message only — a note for the private repository that never reaches
  the public one, in the one command that already makes both. Works with
  `-m` (git already joins repeated `-m` into one message, so this is
  just another one), `-F <file>` (a combined temp file stands in for
  the private half only; the public half keeps reading the original
  file), and the plain editor form (the private commit is amended right
  after it's made — allowed, since it's the commit from the line above,
  not existing history — while the public commit still reuses the
  message from before that amendment). Refused together with anything
  else that skips the editor without a literal message to attach to
  (`--fixup`, `--squash`, `--reuse-message`, ...), and when nothing is
  staged privately in the first place — there'd be no private commit for
  the text to land on.
- `.ppgitignore` is live: before every command, ppgit regenerates a managed
  block in each git-dir's `info/exclude` — the public one hides `.ppgit/`,
  `.ppgitignore` itself and everything the list names; the private one only
  hides `.ppgit/` (it's the superset, it tracks everything else). Edits to
  `.ppgitignore` take effect on the very next command. `info/exclude` is
  used rather than `.gitignore` because it's never committed, so the public
  repository gives away neither the private half's existence nor which
  paths are private. Anything you wrote in `info/exclude` yourself is left
  alone.
- ppgit warns when the public repository still tracks a file that
  `.ppgitignore` now lists — excluding a path only hides it while it's
  untracked, so a file committed publicly *before* being listed keeps going
  out with every push. Ordinary commands still run (with the warning), but
  `push` is refused until it's resolved, and ppgit prints the
  `git rm --cached` lines needed to fix it.
- That refusal is also enforced outside ppgit: `init` and `clone` install
  a `pre-push` hook into the public repository, so a raw `git push` that
  never went through ppgit is stopped by the same check. The hook is
  self-contained — it reads `.ppgitignore` directly, so it stays correct
  however long ago ppgit last ran — and ppgit never overwrites a hook it
  didn't write itself. `doctor` checks the hook is in place (including
  the silent failure mode of a hook that lost its executable bit).
- `ppgit privatize <path>...` / `ppgit publicize <path>...` move paths
  across the line in one command. `privatize` lists each path in
  `.ppgitignore` and, if it was already committed publicly, untracks it
  there (`git rm --cached` — the file stays on disk and in the private
  repository), leaving the staged removal for you to commit. `publicize`
  removes a path from the list, so the public repository can see it
  again — publishing it is then an ordinary `add` and `commit`.
- `ppgit doctor` checks, in one command, that the two halves are in step,
  and reports rather than repairs — every finding comes with the command
  that fixes it. It first fetches both remotes (being offline isn't fatal,
  it just says the comparisons are against what was last seen), then looks
  at: both halves being on the same branch; each having an `origin`, and
  one pointing where ppgit itself would point it today (a remote in the
  wrong protocol fails every push, and `init` leaves an existing `origin`
  alone however wrong it is); each having a fetch refspec, without which no
  branch can have an upstream and `pull` is reduced to guessing; how each
  stands against its remote, where being ahead or behind is merely noted
  but having *diverged* is a problem, since neither `push` nor `pull`
  settles it; the **superset invariant** — that the private repository
  holds every file the public one tracks, at the same content — which is
  what a public-only `pull` quietly breaks; and files still tracked
  publicly despite `.ppgitignore` listing them. The exit code is non-zero
  when anything is actually wrong, so it can gate a script.

  `ppgit doctor --fix` additionally runs the repairs that have exactly
  one right answer — restoring a missing `origin` (when `gh` can say
  what it should be) or fetch refspec, re-pointing an `origin` at the
  URL ppgit would use, re-connecting a severed upstream, staging the
  untracking of files listed as private, reinstalling the pre-push hook
  — and prints what it ran. Anything that needs a judgment call (a
  divergence, a broken superset invariant) is still only reported, and
  a commit is always left to the user. One repair can uncover the next
  fixable thing, so `--fix` says when a re-run is worth it.
- Every successful dual `commit` — both halves genuinely committing
  something new, not the ordinary case of one side having nothing to do —
  is recorded with a pair of git notes: the private commit gets a note
  holding the public SHA, the public commit one holding the private SHA.
  Neither note needs the SHA it names to actually exist as an object in
  that repository, which it usually doesn't (the two object stores hold
  different commits). A commit with no such note is, by construction, not
  a paired one — the distinction `reset` (below) is built on. The notes
  travel too: `init` and `clone` add a fetch refspec for them
  (additively, so it sits alongside the branch one rather than replacing
  it), and `push` follows up an ordinary push with a second, quiet one
  for just the notes ref, since git doesn't include `refs/notes/*` in
  either direction by default.
- `ppgit reset <target>` is the first command routed by that pairing.
  `target` is classified first: **shared** (paired — the note says so)
  resets both repositories to their respective halves of the pair,
  whichever side `target` was named from (its public SHA, its private
  SHA, a branch name, `HEAD~2`, ...) — the two repositories never hold
  each other's objects even for a paired commit, so naming one by its
  *other* half's SHA is exactly what makes this classification necessary
  rather than a simple existence check. **Private-only** (no note)
  resets the private repository to `target` itself and the public one to
  the nearest shared commit *before* it — the closest the public half
  can get to "the same point in history" when `target` has nothing there
  at all. An unpaired *public* commit (predates the pairing mechanism, or
  is the orphan `doctor` already knows about) is refused rather than
  guessed at, pointing at `doctor`. Only the unambiguous "reset to a
  commit" form is routed this way — no target, `reset -- <path>`, or
  several paths pass through exactly as before; `--public`/`--private`
  still work as a literal, unclassified single-half reset when that's
  genuinely what's wanted.
- `ppgit cherry-pick <target>` is classified the same way `reset` is.
  **Shared**: cherry-picked into both repositories, private using its own
  SHA of the pair — which may carry private-only content the public diff
  never had — public using its; the two resulting commits are then paired
  the same way a dual `commit` pairs its own two, since this is the same
  logical change landing in both places again. The public half isn't
  actually run through a real `git cherry-pick`: doing that would hit the
  same "would overwrite local changes" problem `pull` (and `checkout`,
  see below) ran into, since by then the private cherry-pick has already
  rewritten the shared working tree out from under the public index.
  Instead, once the private half succeeds, the public half stages
  whatever the tree now holds for the paths it tracks and commits that
  with the target's own message and authorship — equivalent to what a
  real cherry-pick there would have produced, arrived at without hitting
  the same wall. **Private-only**: cherry-picked into the private
  repository alone — the public repository was never going to receive
  this content, so there's nothing for it to do. An unpaired *public*
  target is refused, exactly like `reset`.
- `ppgit rebase <upstream>` is the last of the three, and the only one
  that replays a whole *range* rather than acting on one commit. It runs
  as a small transaction: the private half does a real rebase (it always
  replays everything, shared and private-only alike), then the public
  half is reconstructed entirely in plumbing — never checked out to
  anywhere, since the shared working tree already holds the right
  content the moment the private rebase finishes — using the same
  message-and-authorship trick `cherry-pick` uses, once per originally
  shared commit, chained onto the resolved upstream's public side. If
  the replayed range comes out a different length than it went in (git
  silently drops a commit that turns out empty, unless
  `--reapply-cherry-picks` is given — nothing here changes that default),
  positional matching can no longer be trusted, and the private half is
  rolled back to exactly where it stood before — safe and exact, since
  the abandoned commits aren't deleted, just unreferenced. `--force`
  skips that rollback instead: the private half stays rebased and the
  public half is left untouched, a state `pp doctor` can point at for a
  manual repair. A conflict during the real private rebase is left for
  `git --git-dir=.ppgit --work-tree=. rebase --continue`/`--abort` to
  resolve directly; the public half was never touched, so reconciling it
  afterward is an ordinary `pp doctor` / `pp add . && pp commit` job.
  Scoped the same way as `reset`/`cherry-pick`: only the current branch,
  no `--onto`, `-i`, or `--continue`/`--abort`/`--skip` — anything else
  passes through untouched.
- A `pp` alias binary is built alongside `ppgit`.

### Requirements

- **git** — ppgit is a wrapper, not a reimplementation; everything is done
  by calling it.
- **[`gh`](https://cli.github.com/), logged in** — needed by `init`, `clone`
  and (only to verify a remote) `doctor`. Everything else works without it.
  This also means the GitHub half of ppgit is GitHub-only: the local half
  of the split is plain git and cares about nothing else, but ppgit won't
  create or find repositories anywhere but GitHub.
- Developed and used on Linux. Nothing in it is knowingly platform-specific
  beyond reporting a signal-killed git on Unix, but other platforms are
  untested.

### Installation

```sh
cargo install --path .
```

Installs both the `ppgit` and `pp` binaries.

### License

GPL-3.0-or-later, see [LICENSE](LICENSE).

---

## Русский

`ppgit` (private-public git) — CLI-обёртка над `git` для проектов, у которых
есть публичная часть (открытый исходный код, README, документация) и
приватная часть (заметки, `CLAUDE.md` и всё, что не должно быть видно
посторонним).

Идея: одна рабочая директория, но два репозитория и два GitHub-remote'а —
обычный публичный (`repo-name`) и приватный, который содержит публичную часть
плюс приватные файлы (`pp-repo-name`). `ppgit` берёт на себя рутину
синхронизации между ними.

### Статус

Проект в ранней стадии. `ppgit` — прозрачная обёртка над `git`, плюс
несколько собственных команд:

- `ppgit <command>` пробрасывает всё как есть в `git <command>` (аргументы
  передаются через `args_os`, так что пути в не-UTF-8 не портятся), а наружу
  возвращается настоящий код завершения git, включая случай, когда git убит
  сигналом (на Unix).
- `ppgit -V`/`--version` и `ppgit -h`/`--help`/голый `ppgit` показывают
  собственную версию/краткую справку ppgit — всё остальное по-прежнему
  уходит в `git`.
- `ppgit init` настраивает обе половины разделения на публичное/приватное:
  - локально — создаёт `.git` (публичный, полностью обычный репозиторий),
    если его ещё нет, шаблон `.ppgitignore` (не трогается, если уже
    существует) и приватный git-dir `.ppgit`. В уже существующем проекте
    приватная половина начинается не с нуля, а как bare-*клон* публичной
    истории — так у них общий предок, вместо того чтобы первый приватный
    коммит пересоздавал весь проект на пустом месте;
  - на GitHub (через `gh`, предварительно проверив, что он установлен и
    залогинен) — создаёт публичный (`<имя-директории>`) и приватный
    (`pp-<имя-директории>`) репозитории, если их ещё нет, прописывает
    `origin` в каждом git-dir на соответствующий (по SSH или HTTPS — как
    настроен `gh`) и включает `push.autoSetupRemote`, чтобы первый push
    ветки не требовал `--set-upstream`.

  Каждый шаг безопасно запускать повторно — при втором `ppgit init` ничего
  не пересоздаётся и не дублируется.
- `ppgit clone <репозиторий> [<директория>]` — второй вход в проект: он
  разворачивает уже существующий проект на другой машине, где `ppgit init`
  не от чего отталкиваться. Репозиторий можно назвать любым способом,
  который понимает `gh` — `name`, `owner/name` или URL, — и это может быть
  *любая* из двух половин: ppgit смотрит на её видимость, вычисляет парную
  (`name` ↔ `pp-name`) и отказывается работать, если пары нет, вместо того
  чтобы наполовину собрать проект из репозитория, у которого приватной
  стороны никогда и не было.

  Публичная половина клонируется обычным образом, приватная — как
  bare-git-dir `.ppgit` поверх того же рабочего дерева; само дерево затем
  берётся из приватного HEAD, поскольку он superset. Bare-клону нужно
  вернуть две вещи, которых `git clone --bare` не настраивает: fetch-refspec
  (без него ни у одной ветки нет upstream, так что `pull` вынужден
  догадываться, а `status` никогда не покажет ahead/behind) и индекс (без
  него все отслеживаемые файлы читаются как удалённые). `ppgit clone` делает
  и то, и другое, так что в результате получается проект, в котором сразу
  можно делать `pull` и `commit`. Если половины приехали на разных ветках по
  умолчанию, об этом сообщается: рабочее дерево у них одно, и ppgit нужно,
  чтобы ветки совпадали. А если пара разошлась ещё *на remote'ах* —
  публичные файлы, которых приватный репозиторий не содержит или содержит
  в устаревшем виде, — clone скажет об этом сразу, не оставляя находку
  первому запуску `doctor`.
- Команды маршрутизируются в один репозиторий или в оба. `add`,
  `status`, `rm`, `mv`, `restore`, `push` и `fetch` по умолчанию
  идут в оба; всё остальное описывает историю, которую два репозитория
  вправе иметь разную, и уходит в публичный — чтобы голый `ppgit log`
  показывал то же, что показал бы голый `git log`. Ведущий `--public`,
  `--private` или `--both` это переопределяет. При дуальном запуске
  приватный (суперсет) идёт первым и один определяет код возврата:
  публичный по построению видит не все файлы, поэтому «ему нечего делать»
  — штатный исход, а не ошибка. `commit` — особый случай: он не просто
  по умолчанию идёт в оба, см. ниже, он вообще отказывается сужаться.
- `ppgit pull` оркестрируется, а не зеркалируется: два обычных pull
  дерутся за общее рабочее дерево — та половина, что успела первой,
  обновила бы его, и merge второй отказался бы «перезаписывать локальные
  изменения» (git меряет их по своему индексу, так что файл, уже
  содержащий ровно результат merge, не спасает). Поэтому приватная
  (superset) половина делает настоящий pull — её merge и есть то, что
  должно лежать в дереве, — а публичная затем догоняется, не касаясь
  дерева: fetch и fast-forward только ссылки и индекса, под охраной
  проверки на предка, так что ничего никогда не теряется. Публичная
  половина, действительно разошедшаяся со своим remote, остаётся ровно
  как была — об этом сообщается. Явный `--public`/`--private` pull
  остаётся прозрачным пробросом.
- `stash` и `clean` маршрутизируются отдельно: обе команды касаются всего
  общего рабочего дерева, которое целиком видит только приватная
  (superset) половина. `stash` по умолчанию идёт в приватную — её stash
  полный, тогда как публичный молча оставил бы изменения приватных
  файлов на месте (`--public` по-прежнему сужает намеренно; `--both`
  отклоняется: та половина, что застэшила бы первой, унесла бы
  изменения, которые собиралась сохранить вторая). `stash --all`
  отклоняется всегда: он стэшит игнорируемые файлы, приватный git-dir
  сам игнорируется, и git застэшил бы репозиторий внутрь самого себя и
  удалил его — проверено, к сожалению. `clean` выполняется только на
  приватной половине, чьё множество неотслеживаемых файлов — настоящее
  (для публичной каждый приватный файл — просто «игнорируемый», так что
  публичный `clean -x` удалил бы их все), а сам приватный git-dir ppgit
  прикрывает добавленным `-e`. Вне ppgit-проекта обе команды уходят в
  git нетронутыми.
- Команды работы с ветками (`branch`, `checkout`, `switch`, `merge`)
  всегда идут в оба и отклоняют `--public`/`--private`. Репозитории делят
  одно рабочее дерево, поэтому ветка, существующая только в одном, — или
  два репозитория на разных ветках — отправит следующий коммит туда, куда
  второй не сможет последовать. (`checkout -- <путь>` — файловая
  операция, а не работа с ветками, и принимает флаги scope как обычно.)
  Переключение на ветку, у которой отслеживаемые файлы реально
  отличаются от текущей, — это больше, чем «выполнить дважды»: наивный
  двойной checkout упирается в ту же стену «перезаписи локальных
  изменений», что описана ниже для `pull` и `cherry-pick`, — приватный
  checkout переписывает общее рабочее дерево раньше, чем публичный
  получает свою очередь. `ppgit checkout <ветка>` (единственное голое имя
  ветки, которая уже есть в обеих половинах) в эту стену не упирается:
  настоящий checkout выполняется только в приватной половине; публичная
  подтягивается переключением ref'а и синхронизацией индекса, которые
  вообще не трогают рабочее дерево, вместо второго настоящего checkout.
  Всё, что этот механизм не обрабатывает специально (`-b`, несколько
  аргументов, произвольный commit-ish), по-прежнему проходит через
  обычный двойной вызов.
- `mv` по умолчанию идёт в оба, но не как два `git mv`: `git mv` двигает
  файл и на диске, так что второй вызов уже не нашёл бы исходного файла.
  Настоящий `git mv` выполняется только в приватной половине, а публичная
  получает то же переименование только в индексе (`update-index`) — для
  тех файлов, которые она действительно отслеживает: при перемещении
  каталога с приватными файлами внутри публично индексируются только
  публичные. Зеркалируется лишь то, что приватный `git mv` действительно
  сделал, так что `-n` и `-k` работают как обычно.
- `ppgit commit` открывает редактор один раз, а не по разу на
  репозиторий: сначала интерактивно делается приватный коммит, затем его
  сообщение дословно переиспользуется для публичного. Если сообщение
  задано явно (`-m`, `-F`, `--fixup`, ...), переиспользовать нечего — оба
  получают одинаковые аргументы. Половина, где ничего не застейджено,
  пропускается, а не считается ошибкой — включая обратный от привычного
  случай: если застейджено только в публичном индексе (например, после
  `privatize`), обычный `pp commit` коммитит только туда, поскольку
  приватной половине нечего было делать. `commit` — в отличие от всего
  остального на этой странице — вообще отказывается принимать
  `--public`/`--private`: коммит, сделанный с сужением, происходит в
  отдельном вызове от своей возможной пары, так что нет ни одного
  момента, где эти две половины можно было бы связать как одно
  логическое изменение. Ничего не теряется: случаи, для которых раньше
  использовался суженный коммит (изменение затрагивает только одну
  половину), и так корректно коммитятся обычной, несуженной формой,
  описанной выше.
- `pp commit --private <текст>` добавляет `<текст>` только в сообщение
  приватного коммита — заметка для приватного репозитория, которая
  никогда не попадает в публичный, в той же единственной команде, что и
  делает оба коммита. Работает с `-m` (git и так склеивает повторные
  `-m` в одно сообщение, так что это просто ещё один), с `-F <файл>`
  (комбинированный временный файл подменяет источник сообщения только
  для приватной половины; публичная по-прежнему читает исходный файл) и
  с обычной редакторской формой (приватный коммит сразу после создания
  амендится — это разрешено, поскольку это коммит с предыдущей строки, а
  не существующая история, — тогда как публичный коммит переиспользует
  сообщение до этого amend'а). Отклоняется вместе с чем-либо ещё, что
  обходит редактор без буквального текста сообщения, к которому можно
  было бы что-то добавить (`--fixup`, `--squash`, `--reuse-message`,
  ...), а также когда в приватном индексе вообще ничего не застейджено —
  тогда просто нет приватного коммита, на который лёг бы этот текст.
- `.ppgitignore` работает вживую: перед каждой командой ppgit
  перегенерирует управляемый блок в `info/exclude` каждого git-dir —
  публичный прячет `.ppgit/`, сам `.ppgitignore` и всё, что перечислено в
  списке; приватный прячет только `.ppgit/` (он суперсет, всё остальное
  отслеживает). Правки `.ppgitignore` подхватываются следующей же
  командой. Используется именно `info/exclude`, а не `.gitignore`, потому
  что он никогда не коммитится — публичный репозиторий не выдаёт ни факт
  существования приватной половины, ни то, какие пути приватные. Строки,
  которые вы написали в `info/exclude` сами, не затрагиваются.
- ppgit предупреждает, если публичный репозиторий всё ещё отслеживает
  файл, который теперь перечислен в `.ppgitignore`: исключение прячет путь
  только пока он неотслеживаемый, поэтому файл, закоммиченный публично
  *до* попадания в список, продолжит уезжать с каждым push. Обычные
  команды при этом выполняются (с предупреждением), а `push` отклоняется
  до устранения конфликта — ppgit печатает готовые строки
  `git rm --cached` для исправления.
- Этот отказ действует и вне ppgit: `init` и `clone` устанавливают в
  публичный репозиторий hook `pre-push`, так что голый `git push` в обход
  ppgit останавливается той же проверкой. Hook самодостаточен — он читает
  `.ppgitignore` напрямую и остаётся корректным, как бы давно ppgit ни
  запускался, — и ppgit никогда не перезаписывает hook, написанный не им.
  `doctor` проверяет, что hook на месте (включая тихий режим отказа —
  hook, потерявший бит исполняемости).
- `ppgit privatize <путь>...` / `ppgit publicize <путь>...` переносят
  пути через границу одной командой. `privatize` вписывает каждый путь в
  `.ppgitignore` и, если тот уже был закоммичен публично, снимает его там
  с отслеживания (`git rm --cached` — файл остаётся на диске и в
  приватном репозитории), оставляя подготовленное удаление вам на коммит.
  `publicize` убирает путь из списка, после чего публичный репозиторий
  снова может его видеть — публикация дальше — это обычные `add` и
  `commit`.
- `ppgit doctor` одной командой проверяет, что половины не разъехались, и
  при этом сообщает, а не чинит — к каждой находке прилагается команда,
  которая её исправляет. Сначала он делает fetch в обе половины (отсутствие
  сети не фатально, просто в отчёте будет сказано, что сравнение идёт с
  последним известным состоянием), а затем смотрит: обе ли половины на одной
  ветке; есть ли у каждой `origin` и указывает ли он туда, куда ppgit
  направил бы его сегодня (remote не в том протоколе роняет любой push, а
  `init` не трогает уже существующий `origin`, каким бы неправильным тот ни
  был); есть ли fetch-refspec, без которого ни у одной ветки не может быть
  upstream, а `pull` вынужден догадываться; как каждая половина соотносится
  со своим remote — отставание и опережение просто отмечаются, а вот
  **расхождение** это проблема, потому что ни `push`, ни `pull` его сами не
  разрешат; **superset-инвариант** — что приватный репозиторий содержит все
  файлы, отслеживаемые публичным, и в том же состоянии, — который молча
  ломает публичный `pull`; и файлы, всё ещё отслеживаемые публично вопреки
  `.ppgitignore`. Код возврата ненулевой, только если что-то действительно
  не так, — так что doctor годится в качестве проверки в скрипте.

  `ppgit doctor --fix` дополнительно выполняет починки, у которых ровно
  один правильный ответ: восстанавливает отсутствующий `origin` (когда
  `gh` может сказать, каким он должен быть) или fetch-refspec,
  перенаправляет `origin` на URL, который выбрал бы ppgit, заново
  подключает оборванный upstream, ставит в индекс снятие с отслеживания
  файлов, перечисленных как приватные, переустанавливает pre-push hook —
  и печатает, что именно выполнил. Всё, что требует человеческого решения
  (расхождение с remote, сломанный superset-инвариант), по-прежнему
  только сообщается, а коммит всегда остаётся за пользователем. Одна
  починка может вскрыть следующую — `--fix` говорит, когда стоит
  запуститься ещё раз.
- Каждый успешный дуальный `commit` — когда обе половины реально
  закоммитили что-то новое, а не обычный случай, когда одной из них
  нечего было делать, — фиксируется парой git-notes: приватный коммит
  получает заметку с публичным SHA, публичный — заметку с приватным SHA.
  Ни одной из заметок не требуется, чтобы названный ею SHA существовал в
  этом репозитории как объект, — обычно он там и не существует (у двух
  хранилищ объектов разные коммиты). Это задел на будущее: коммит без
  такой заметки по построению не является парным — именно на этом
  различии построен `reset` (ниже). Заметки ещё и переносятся между
  машинами: `init` и `clone` добавляют для них fetch-refspec (именно
  добавляют, а не заменяют, так что он встаёт рядом со refspec'ом для
  веток, а не вместо него), а `push` следом за обычным push выполняет
  ещё один, тихий, только для ref'а с заметками — git по умолчанию не
  переносит `refs/notes/*` ни в одну сторону.
- `ppgit reset <target>` — первая команда, маршрутизируемая по этой
  парности. `target` сначала классифицируется: **парный** (есть заметка)
  сбрасывает оба репозитория на их половины пары, с какой бы стороны
  `target` ни был назван — по своему публичному SHA, приватному SHA,
  имени ветки, `HEAD~2`, — два репозитория никогда не хранят объекты
  друг друга даже для парного коммита, поэтому называние одного по SHA
  *другой* половины — ровно то, что делает эту классификацию нужной, а
  не просто проверкой существования объекта. **Только приватный** (нет
  заметки) сбрасывает приватный репозиторий на сам `target`, а публичный
  — на ближайший парный коммит *перед* ним: максимально близкая для
  публичной половины точка к «тому же месту в истории», когда у `target`
  там вообще ничего нет. Непарный *публичный* коммит (появился до
  механизма парности либо тот самый сирота, который уже знает `doctor`)
  отклоняется, а не угадывается, — с отсылкой к `doctor`. Так
  маршрутизируется только однозначная форма «сбросить на коммит»: без
  цели, `reset -- <путь>` или несколько путей проходят насквозь как и
  раньше; `--public`/`--private` по-прежнему работают как буквальный,
  неклассифицированный reset одной половины, когда именно это и нужно.
- `ppgit cherry-pick <target>` классифицируется так же, как `reset`.
  **Парный**: применяется к обоим репозиториям — приватный своим SHA
  пары (в нём может быть приватный контент, которого не было в публичном
  диффе), публичный своим; два получившихся коммита затем связываются
  парой заметок точно так же, как это делает дуальный `commit`, — это же
  логическое изменение снова приземляется в оба места. Публичная
  половина при этом намеренно НЕ выполняется настоящим
  `git cherry-pick`: это упёрлось бы в ту же проблему «перезаписи
  локальных изменений», что и `pull` (и `checkout`, см. ниже), — к этому
  моменту приватный cherry-pick уже переписал общее рабочее дерево из-под
  публичного индекса. Вместо этого, как только приватная половина
  успешно завершилась, публичная застейджит всё, что сейчас лежит в
  дереве для отслеживаемых ею путей, и закоммитит это с сообщением и
  авторством исходной цели — эквивалент того, что дал бы настоящий
  cherry-pick там же, но полученный в обход той же стены. **Только
  приватный**: применяется только к приватному репозиторию — публичному
  всё равно нечего было получать, раз этот контент туда изначально не
  предназначался. Непарная *публичная* цель отклоняется точно так же,
  как у `reset`.
- `ppgit rebase <upstream>` — последняя из трёх и единственная, что
  воспроизводит целый *диапазон*, а не действует на одном коммите.
  Работает как небольшая транзакция: приватная половина делает настоящий
  rebase (она всегда воспроизводит всё, и парное, и только-приватное),
  затем публичная половина реконструируется целиком в плюмбинге —
  никогда никуда не чекаутится, поскольку общее рабочее дерево к моменту
  завершения приватного rebase уже держит нужное содержимое, — тем же
  трюком с сообщением и авторством, что использует `cherry-pick`, по
  разу на каждый изначально парный коммит, цепочкой поверх публичной
  стороны разрешённого upstream. Если воспроизведённый диапазон вышел
  другой длины, чем был (git молча роняет коммит, ставший пустым, если
  не передан `--reapply-cherry-picks` — здесь это умолчание не
  меняется), позиционному соответствию больше нельзя доверять, и
  приватная половина откатывается ровно туда, где была до rebase, —
  безопасно и точно, поскольку заброшенные коммиты не удаляются, а
  просто становятся недостижимыми. `--force` пропускает этот откат:
  приватная половина остаётся перебазированной, а публичная — нетронутой,
  состояние, на которое может указать `pp doctor` для ручного
  восстановления. Конфликт при настоящем приватном rebase оставляется
  для `git --git-dir=.ppgit --work-tree=. rebase --continue`/`--abort`,
  который решает его напрямую; публичная половина при этом вообще не
  трогалась, так что привести её в соответствие потом — обычная задача
  `pp doctor` / `pp add . && pp commit`. Область действия та же, что у
  `reset`/`cherry-pick`: только текущая ветка, без `--onto`, `-i` или
  `--continue`/`--abort`/`--skip` — всё остальное проходит насквозь
  нетронутым.
- Рядом с `ppgit` собирается алиас `pp`.

### Требования

- **git** — ppgit это обёртка, а не переписанный git; вся работа делается
  вызовами git.
- **[`gh`](https://cli.github.com/), с выполненным входом** — нужен для
  `init`, `clone` и (только чтобы проверить remote) `doctor`. Всё остальное
  работает и без него. Отсюда же следует, что GitHub-половина ppgit
  умеет только GitHub: локальная часть разделения — обычный git, которому
  всё равно, но создавать и находить репозитории ppgit будет только на
  GitHub.
- Разрабатывается и используется под Linux. Ничего заведомо
  платформозависимого, кроме сообщения об убитом сигналом git на Unix, в
  нём нет, но на других платформах не проверялось.

### Установка

```sh
cargo install --path .
```

Ставит сразу оба бинарника — `ppgit` и `pp`.

### Лицензия

GPL-3.0-or-later, см. [LICENSE](LICENSE).
