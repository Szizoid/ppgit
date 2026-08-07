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
  tree, so ppgit needs them in step.
- Commands are routed to one repository or both. `add`, `commit`, `status`,
  `rm`, `mv`, `restore`, `push`, `pull` and `fetch` run against both by
  default; everything else describes history, which the two are entitled to
  disagree about, and goes to the public one — so a bare `ppgit log` shows
  what a bare `git log` would. A leading `--public`, `--private` or
  `--both` overrides that. In dual runs the private (superset) repository
  goes first and alone decides the exit code: the public one deliberately
  can't see every file, so it having nothing to do is an ordinary outcome,
  not a failure.
- Branch commands (`branch`, `checkout`, `switch`, `merge`) always run
  against both and refuse `--public`/`--private`. The two repositories
  share one working tree, so a branch existing in only one of them — or
  the two sitting on different branches — would send the next commit
  somewhere the other can't follow. (`checkout -- <path>` is a file
  operation rather than a branch one, and takes scope flags as usual.)
- `ppgit commit` opens the editor once, not once per repository: the
  private commit is made first, interactively, and its message is reused
  verbatim for the public one. With an explicit `-m` (or `-F`, `--fixup`,
  ...) there's nothing to share and both simply get the same arguments.
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
- A `pp` alias binary is built alongside `ppgit`.

### Plans (not finalized)

- Commands that address a specific commit (`rebase`, `cherry-pick`,
  `reset <sha>`) currently go to the public repository only. The two
  repositories hold genuinely different commits, so these can't simply be
  mirrored — what they should do in dual mode is still an open question.

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
  чтобы ветки совпадали.
- Команды маршрутизируются в один репозиторий или в оба. `add`, `commit`,
  `status`, `rm`, `mv`, `restore`, `push`, `pull` и `fetch` по умолчанию
  идут в оба; всё остальное описывает историю, которую два репозитория
  вправе иметь разную, и уходит в публичный — чтобы голый `ppgit log`
  показывал то же, что показал бы голый `git log`. Ведущий `--public`,
  `--private` или `--both` это переопределяет. При дуальном запуске
  приватный (суперсет) идёт первым и один определяет код возврата:
  публичный по построению видит не все файлы, поэтому «ему нечего делать»
  — штатный исход, а не ошибка.
- Команды работы с ветками (`branch`, `checkout`, `switch`, `merge`)
  всегда идут в оба и отклоняют `--public`/`--private`. Репозитории делят
  одно рабочее дерево, поэтому ветка, существующая только в одном, — или
  два репозитория на разных ветках — отправит следующий коммит туда, куда
  второй не сможет последовать. (`checkout -- <путь>` — файловая
  операция, а не работа с ветками, и принимает флаги scope как обычно.)
- `ppgit commit` открывает редактор один раз, а не по разу на
  репозиторий: сначала интерактивно делается приватный коммит, затем его
  сообщение дословно переиспользуется для публичного. Если сообщение
  задано явно (`-m`, `-F`, `--fixup`, ...), переиспользовать нечего — оба
  получают одинаковые аргументы.
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
- Рядом с `ppgit` собирается алиас `pp`.

### Планы (не финализировано)

- Команды, адресующие конкретный коммит (`rebase`, `cherry-pick`,
  `reset <sha>`), сейчас уходят только в публичный репозиторий. У двух
  репозиториев принципиально разные коммиты, поэтому просто зеркалить их
  нельзя — что они должны делать в дуальном режиме, пока открытый вопрос.

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
