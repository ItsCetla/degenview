# Cache architecture

Regenerable data lives in `store::cache_dir`, never the user-data folder: macOS
`~/Library/Caches/com.degenview.rust`, Windows `%LOCALAPPDATA%\com.degenview.rust\Cache`, and Linux
`$XDG_CACHE_HOME` or `~/.cache`. `DEGENVIEW_CACHE_DIR` overrides it. Missing, unreadable, or invalid
files are misses.

| Path | Data | Owner |
| --- | --- | --- |
| `images/<prefix>/<url-hash>` | Logo bytes | `image_loader` |
| `lookups/<namespace>.json` | Icon URLs, CoinGecko index, provider catalogs | `lookup_cache` |
| `fx/*.json` | Fiat and BTC history | `fx_rates` |

- `CachedImageLoader` serves memory → disk → dedicated network workers. After 30 days it shows stale
  data while refreshing; downloaded bodies must identify as images; failures retry after three
  minutes.
- `icon` caches resolved artwork for 30 days, clean misses for one day, and never caches failures.
  CoinGecko's top 250 and names last one day.
- Binance, Coinbase, and Alpaca catalogs refresh daily, search locally, and serve stale on error.
- Ten seconds after launch, `disk_cache::prune` removes images older than 90 days, then oldest files
  until below 75% of 128 MB. Lookup entries expire after 90 days.
- Settings ▸ Storage clears only these cache areas, increments the write generation, drops all
  in-memory layers, and forgets egui textures off the UI thread. In-flight old-generation writes
  are discarded.
- Candle cache (`service::cache`) is memory-only, keyed by source/symbol/interval with per-source
  TTLs. CoinGecko and GeckoTerminal share per-host rate limiters.
