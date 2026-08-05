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

The project is at a very early stage — the public/private split logic isn't
implemented yet. Right now `ppgit` is a transparent passthrough wrapper over
`git`:

- `ppgit <command>` forwards everything as-is to `git <command>` (arguments
  are passed through via `args_os`, so non-UTF-8 paths aren't mangled), and
  the real exit code is propagated back, including the case where `git` gets
  killed by a signal (on Unix).
- `ppgit`/`ppgit -V`/`ppgit --version` print ppgit's own version, `ppgit -h`
  /`--help`/a bare `ppgit` print ppgit's own short help — everything else
  still goes straight to `git`.
- A `pp` alias binary is built alongside `ppgit` (same binary, installed
  under a second name — see `src/bin/pp.rs`).

### Plans (not finalized)

- `.ppgitignore` — a list of paths that should only go into the private
  repository.
- A dedicated state directory `.ppgit/`.
- Commands to set up and sync both remotes.

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

Проект в самой ранней стадии — логика разделения на публичное/приватное ещё
не реализована. Пока `ppgit` — прозрачная обёртка над `git`:

- `ppgit <command>` пробрасывает всё как есть в `git <command>` (аргументы
  передаются через `args_os`, так что пути в не-UTF-8 не портятся), а
  наружу возвращается настоящий код завершения git, включая случай, когда
  git убит сигналом (на Unix).
- `ppgit -V`/`--version` и голый `ppgit`/`-h`/`--help` показывают
  собственную версию/справку ppgit — всё остальное по-прежнему уходит в
  `git`.
- Рядом с `ppgit` собирается алиас `pp` (тот же код, второй бинарник — см.
  `src/bin/pp.rs`).

### Планы (не финализировано)

- `.ppgitignore` — список путей, которые должны попадать только в приватный
  репозиторий.
- Собственная директория состояния `.ppgit/`.
- Команды для настройки и синхронизации обоих remote'ов.

### Установка

```sh
cargo install --path .
```

Ставит сразу оба бинарника — `ppgit` и `pp`.

### Лицензия

GPL-3.0-or-later, см. [LICENSE](LICENSE).
