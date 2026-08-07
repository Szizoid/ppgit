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
    exists), a bare `.ppgit` git-dir (the private, superset repository), and
    idempotently adds `.ppgit`/`.ppgitignore` to `.gitignore`;
  - on GitHub (via `gh`, after checking it's installed and logged in) —
    creates the public (`<dir-name>`) and private (`pp-<dir-name>`)
    repositories if they don't already exist, and points `origin` in each
    git-dir at the right one.

  Every step is safe to run more than once — nothing gets recreated or
  duplicated on a second `ppgit init`.
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
- A `pp` alias binary is built alongside `ppgit`.

### Plans (not finalized)

- Routing regular commands (`add`, `commit`, `status`, `push`, `pull`, ...)
  to both repositories at once, instead of only the public one.

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
    существует), bare-репозиторий `.ppgit` (приватный, суперсет), и
    идемпотентно добавляет `.ppgit`/`.ppgitignore` в `.gitignore`;
  - на GitHub (через `gh`, предварительно проверив, что он установлен и
    залогинен) — создаёт публичный (`<имя-директории>`) и приватный
    (`pp-<имя-директории>`) репозитории, если их ещё нет, и прописывает
    `origin` в каждом git-dir на соответствующий.

  Каждый шаг безопасно запускать повторно — при втором `ppgit init` ничего
  не пересоздаётся и не дублируется.
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
- Рядом с `ppgit` собирается алиас `pp`.

### Планы (не финализировано)

- Маршрутизация обычных команд (`add`, `commit`, `status`, `push`,
  `pull`, ...) сразу в оба репозитория, а не только в публичный.

### Установка

```sh
cargo install --path .
```

Ставит сразу оба бинарника — `ppgit` и `pp`.

### Лицензия

GPL-3.0-or-later, см. [LICENSE](LICENSE).
