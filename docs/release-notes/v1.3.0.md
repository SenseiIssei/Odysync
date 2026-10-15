# Odysync 1.3.0

Focus: faster scans and clear feedback during updates, plus import/export of app profiles.

Highlights
- Progress animation during installs so updates never look frozen
- Upgrade-scan cache with configurable TTL (default 15 min)
- Export/import profiles from JSON
- Open Microsoft Store → Library to trigger Store updates manually
- Logs for every run in `%LOCALAPPDATA%\Odysync\logs\`

Config
- Settings at `%LOCALAPPDATA%\Odysync\settings.json`
- Defaults include `cache_ttl_minutes`

Examples
- `Odysync.exe --apps`
- `Odysync.exe --export "%USERPROFILE%\\Desktop\\profiles.json"`
- `Odysync.exe --import "%USERPROFILE%\\Desktop\\profiles.json"`

Support
- Ko-fi: https://ko-fi.com/senseiissei