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

The project is at a very early stage. Right now `ppgit` is a literal
passthrough: `ppgit <command>` is equivalent to `git <command>`. The
public/private split logic isn't implemented yet.

### Plans (not finalized)

- `.ppgitignore` — a list of paths that should only go into the private
  repository.
- A dedicated state directory `.ppgit/`.
- Commands to set up and sync both remotes.
- A `pp` alias for `ppgit`.

### Installation

```sh
cargo install --path .
```

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

Проект в самой ранней стадии. Пока `ppgit` — буквальный passthrough:
`ppgit <command>` эквивалентно `git <command>`. Логика разделения на
публичное/приватное ещё не реализована.

### Планы (не финализировано)

- `.ppgitignore` — список путей, которые должны попадать только в приватный
  репозиторий.
- Собственная директория состояния `.ppgit/`.
- Команды для настройки и синхронизации обоих remote'ов.
- Алиас `pp` для `ppgit`.

### Установка

```sh
cargo install --path .
```

### Лицензия

GPL-3.0-or-later, см. [LICENSE](LICENSE).
