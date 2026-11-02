# lockbox

Менеджер паролей с TOTP-кодами. Локальный режим хранит всё в одном
зашифрованном файле `.lbx`. Облачный режим сохраняет изменения на устройстве
и обменивается зашифрованными подписанными событиями через Storj.

## Development and documentation

From the repository root, run `cargo run --locked` to start the desktop app.
See the [developer guide](docs/development.md) for validation and asset generation,
and the [documentation index](docs/README.md) for architecture, implemented sync
behavior, recovery and release plans.

## Browser companion

Chrome and Firefox extensions open the desktop app through a deep link. Unlock
with the normal password or Touch ID flow, then choose an exact-origin account
to fill. The extension only proxies the request and fills the approved login;
master passwords and vault files stay in the desktop app. See the
[installation and security guide](ext/README.md).

## Website icon privacy and resource limits

Bundled icons are used by default. **Settings → Security → Download website icons**
opts into contacting the selected record's website. Downloads use the exact HTTPS
origin, public IP addresses pinned for the connection, no redirects, and no
system proxy. Website-provided cross-origin icons are ignored. Icons are cached
unencrypted in the OS cache directory at `lockbox/favicons/v1`, using hashed
filenames. The cache survives locking and restarting: icons expire after 30 days,
and failed lookups retry after five minutes. Unix cache directories use `0700`
permissions and files use `0600`. Cache failures leave downloaded icons usable.

Vault reads and writes are limited to 256 MiB; identity files to 64 KiB. Argon2id
keeps its creation default of 256 MiB and three passes. Readers accept at most
512 MiB, six passes, and 768 MiB-passes of combined work. Files with higher custom
costs are rejected before hashing; they need migration using a trusted compatible
client. Truncated ciphertext and nonregular files are rejected before hashing.
The complete Argon2 working memory is wiped after use.

## Файлы в записи

В редакторе записи выберите «Добавить файл». Доступны до 16 файлов,
суммарно до 1 МиБ на запись; изменения применяются кнопкой «Сохранить».
В просмотре записи «Сохранить файл» выгружает выбранный файл в новое место
на диске. Существующие файлы не перезаписываются.

Имена и байты вложений шифруются вместе с записью: в локальном `.lbx`
и в подписанных событиях облачного обмена. Отдельные незашифрованные копии
не создаются. Выгруженные пользователем файлы остаются обычными файлами.
Удаление вложения сохраняет прежние версии в резервной копии и истории обмена.
Для обмена вложениями обновите все устройства: старые клиенты отклоняют события
с неизвестными полями. Локальные файлы теперь записываются как v6, облачные
профили — v7; чтение прежних v1–v5 поддерживается. Старые приложения новые
файлы не открывают, чтобы не потерять вложения при сохранении.

## Облачный профиль: идентичность и ключи

«Восстановить из Storj» на стартовом экране находит ваши пространства по прежним
12 словам и доступу к bucket без старого `.lbx`. Выберите пространство, новый
локальный пароль и явно подтвердите доверие к доступной облачной версии.
Восстановление создаёт новое устройство владельца и отзывает прежний контроллер.
Существующий профиль сначала откройте мастер-паролем и дождитесь завершения
синхронизации, чтобы создать капсулу восстановления. Credentials нужны также для
префикса `recovery/v1/`. Восстанавливаются только опубликованные данные;
свежесть облачной версии после потери всех независимых копий доказать нельзя.
Подробности и ограничения: [cloud recovery](docs/cloud-recovery.txt).

В создании хранилища выберите многопользовательский режим, заполните настройки
Storj, имя пользователя и имя хранилища. После проверки полей задайте мастер-пароль
(не менее 12 символов) и сохраните 12 слов восстановления. Приложение создаст
локальный профиль `.lbx`. Слова и файл или его резервная копия восстановят доступ
без старого мастер-пароля через «Забыли мастер-пароль?». Отмена выбора файла не создаёт ключи.

Корневой Ed25519-ключ пользователя выводится из 12 слов BIP39 (SLIP-0010).
Приложение создаёт независимые случайные Ed25519-ключи устройства,
HPKE X25519-ключ устройства, случайные ID пространства, namespace, ключ эпохи
и её контекст. Ключи изменений и checkpoint выводятся через HKDF по proposal.
Полные публичные отпечатки доступны на стартовом экране открытого профиля.

Имена, credentials и ключи устройства хранятся внутри зашифрованного файла v8
(Argon2id, HKDF-SHA256, XChaCha20-Poly1305). Корневой ключ дополнительно
зашифрован тем же мастер-паролем с независимой солью. Открытие профиля
паролем или Touch ID не расшифровывает корневой ключ автоматически.
Старые профили сохраняют прежние пароли и не получают слова автоматически.
Подробнее: [слова восстановления и формат v8](docs/recovery-words.md).
При открытом профиле файл `.lbx.lock` блокирует второе открытие; оставшийся
после закрытия lock-файл не означает, что профиль заблокирован.

Новые профили содержат сертификат устройства, подписанный genesis и durable
outbox. Сохранение записи или папки сначала фиксирует локальное событие;
фоновый обмен со Storj не требует доступности сети для локальной работы.
Проверяются подписи, роли, цепочка участников, причинные зависимости и счётчики
устройств. Параллельные версии сохраняются до явного разрешения конфликта.
Изменение состава участников меняет ключ эпохи; поздние изменения остаются
доступны для проверки и переноса.

Общие пространства используют единый экспериментальный протокол без версии, отдельный от полного
proposal v1. API подключения независимых профилей и изменения состава участников
реализован с подписанными приглашением, согласием устройства и подтверждением допуска;
Прежние версии формата обмена не поддерживаются. [Описание API](docs/sync-invitations.md).
Экран «Участники и доступ» создаёт одноразовую ссылку приглашения. Участник
вставляет её в Lockbox, проверяет отдельно полученный код и отправляет заявку.
Администратор видит имя, User ID и Device ID, сверяет их через второй канал
и одобряет конкретное устройство. После согласия участника обмен завершается
автоматически через Storj. Каждому нужны собственные credentials; в ссылке
credentials и ключей хранилища нет. Файловое подключение осталось резервным
способом в раскрывающемся пункте. Роль можно изменить, доступ устройства — отозвать.
Старые bootstrap
профили версии 1 активируются отдельным паролем восстановления с сохранением
записей и пользовательской идентичности, с новыми ключами устройства. Новые
облачные профили с восстановлением из Storj используют bootstrap-схему 6;
первое действие с приглашением по ссылке обновляет профиль до схемы 7. Сохраняется независимый локальный якорь
поколения файла. Откат или отсутствие якоря оставляет записи читаемыми и
останавливает подпись и обмен. Облачные профили схемы 2 открываются для чтения;
для продолжения используется восстановление с новым устройством и повторный допуск.
Подробности: [защита профиля и восстановление](docs/profile-protection.md). Профиль
не загружается в общий bucket и не копируется для подключения другого устройства.
Существующие локальные файлы v1/v2 остаются совместимыми.

Описание границ, API и проверки: [движок синхронизации](docs/sync-engine.md).
Рабочий пример двух клиентов без сети: `cargo run --example shared_space`.
Проверки используют adversarial transport и локальный S3 HTTP fixture;
live-bucket Storj и независимое ревью безопасности пока не выполнены.

## Code navigation

Code is grouped by feature. Start with the behavior you want to change:

```text
src/
  access/          vault creation, unlocking, biometric access and recovery words
  browser/         browser native messaging relay and local callback
  records/         record/folder models, editor, imports, history and views
  sharing/         invitations, joining, participants and sharing views
  sync/            protocol, exchange scheduler and transport providers
  vault/           encrypted files, profiles, recovery and installation protection
  settings/        preferences, config persistence and settings view
  desktop/         app shell, session, windows, shared widgets and translations
  platform/        clipboard, Touch ID, autolock and background jobs
  bin/             asset build tools and browser native host
  compatibility.rs legacy public import paths
  lib.rs           feature modules and public exports
  main.rs          desktop launch
```

Models and protocol rules stay independent of Dioxus and system I/O. Feature
views bind workflows to the desktop contexts; native services stay in `platform`.
Both local and shared modes use the existing `Vault` facade. Shared edits retain
its durable signing-intent and outbox boundaries.

Existing public paths, including `lockbox::vault`, `lockbox::core::sync`, and
`lockbox::app::App`, remain supported. Internal code and examples use feature
paths. See the [developer guide](docs/development.md#module-boundaries) for module
ownership and extension rules.

## Как защищены данные

- Мастер-пароль → Argon2id (256 МиБ, 3 прохода) → HKDF → ключ
  XChaCha20-Poly1305. Шифруется всё содержимое целиком, заголовок файла
  тоже защищён от подмены.
- Сохранение атомарное: временный файл → fsync → переименование;
  предыдущая версия остаётся рядом как `.lbx.bak`.
- Ключ и записи в Rust стираются из памяти при блокировке и закрытии окна.
  При блокировке компоненты с записями и формами удаляются из интерфейса.
  Введённые и показанные секреты также проходят через DOM, JavaScript и
  виртуальный DOM Dioxus: гарантировать стирание этих копий из памяти WebView
  нельзя.
- Скопированный пароль или код помечается для менеджеров буфера как скрытый
  и стирается из буфера через 20 с — если там всё ещё он.
- Автоблокировка: после выбранного в настройках хранилища времени без действий
  (по умолчанию 5 минут), после сна мака дольше этого времени и сразу при
  блокировке экрана. Несохранённая правка записи при этом сбрасывается; за 30 с
  окно предупреждает.

## Архитектурные предложения

[Lockbox Protocol](docs/lockbox-protocol.md) —
подробный проект протокола на английском: идентичности и устройства, приглашения,
подписанные изменения, шифрование, отзыв доступа и обмен через стороннее облако. Описанные
возможности шире реализованного экспериментального протокола без версии;
текущие границы описаны в [движке синхронизации](docs/sync-engine.md).

## Record history and Trash

Open a record and select **Record history** to compare changed fields before and
after each revision. Passwords and TOTP secrets stay hidden until revealed.
Shared history identifies the verified user and device by their full fingerprints;
cloud events have causal ordering but no wall-clock timestamps. Local history
records save times and labels the editor as “Local user”; it does not authenticate
individual people sharing a master password.

Deleting moves a record to **Trash** in the sidebar. Restore preserves the record
ID, secrets and attachments. If its folder was removed, it returns to the root.
History and deleted values remain inside encrypted storage. There is no permanent
purge operation. Local history begins with changes saved by this version; earlier
local edits and deletions cannot be reconstructed. Shared history is limited to
events the device can decrypt; a newly admitted device may lack older revisions.
Concurrent shared versions stay in the existing conflict review until resolved.

Files saved by this version use v6 (local) or v7 (shared); earlier clients reject
them to avoid losing history. Update clients before reopening upgraded files.
