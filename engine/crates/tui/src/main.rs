use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Terminal,
};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone, Copy)]
struct TokenInfo {
    symbol: &'static str,
    decimals: u8,
    is_stable_usd: bool,
    // Optional: approximate USD value per 1 whole token.
    // Used for non-USD stables like EURC.
    usd_per_token: Option<f64>,
}

fn eurc_usd_rate() -> f64 {
    std::env::var("TUI_EURC_USD")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(1.19)
}

fn token_info(network: Option<&str>, address: &str) -> Option<TokenInfo> {
    // Minimal registry for display + USD heuristics.
    // Extend over time (or replace with on-chain metadata lookup later).
    let net = network.unwrap_or("");
    let addr = address.to_lowercase();

    // Treat the zero address as the native token (ETH) for display purposes.
    // Some APIs use this sentinel for native ETH.
    if addr == "0x0000000000000000000000000000000000000000" {
        return Some(TokenInfo {
            symbol: "ETH",
            decimals: 18,
            is_stable_usd: false,
            usd_per_token: None,
        });
    }

    // Ethereum mainnet
    if net == "ethereum-mainnet" {
        return match addr.as_str() {
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48" => Some(TokenInfo {
                symbol: "USDC",
                decimals: 6,
                is_stable_usd: true,
                usd_per_token: Some(1.0),
            }),
            "0xdac17f958d2ee523a2206206994597c13d831ec7" => Some(TokenInfo {
                symbol: "USDT",
                decimals: 6,
                is_stable_usd: true,
                usd_per_token: Some(1.0),
            }),
            "0x6b175474e89094c44da98b954eedeac495271d0f" => Some(TokenInfo {
                symbol: "DAI",
                decimals: 18,
                is_stable_usd: true,
                usd_per_token: Some(1.0),
            }),
            "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2" => Some(TokenInfo {
                symbol: "WETH",
                decimals: 18,
                is_stable_usd: false,
                usd_per_token: None,
            }),
            "0x2260fac5e5542a773aa44fbcfedf7c193bc2c599" => Some(TokenInfo {
                symbol: "WBTC",
                decimals: 8,
                is_stable_usd: false,
                usd_per_token: None,
            }),
            "0x1f9840a85d5af5bf1d1762f925bdaddc4201f984" => Some(TokenInfo {
                symbol: "UNI",
                decimals: 18,
                is_stable_usd: false,
                usd_per_token: None,
            }),
            _ => None,
        };
    }

    // Base mainnet
    if net == "base-mainnet" {
        return match addr.as_str() {
            // USDC on Base
            "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913" => Some(TokenInfo {
                symbol: "USDC",
                decimals: 6,
                is_stable_usd: true,
                usd_per_token: Some(1.0),
            }),
            // EURC on Base
            "0x60a3e35cc302bfa44cb288bc5a4f316fdb1adb42" => Some(TokenInfo {
                symbol: "EURC",
                decimals: 6,
                is_stable_usd: false,
                usd_per_token: Some(eurc_usd_rate()),
            }),
            // WETH on Base
            "0x4200000000000000000000000000000000000006" => Some(TokenInfo {
                symbol: "WETH",
                decimals: 18,
                is_stable_usd: false,
                usd_per_token: None,
            }),
            _ => None,
        };
    }

    None
}

fn short_addr(addr: &str) -> String {
    if addr.len() <= 16 {
        return addr.to_string();
    }
    format!("{}…{}", &addr[..8], &addr[addr.len() - 6..])
}

fn token_label(network: Option<&str>, addr: &str) -> String {
    if let Some(info) = token_info(network, addr) {
        format!("{}:{}", info.symbol, short_addr(addr))
    } else {
        short_addr(addr)
    }
}

fn format_amount_atoms(amount: &str, decimals: Option<u8>) -> String {
    let amount = amount.trim();
    if amount.is_empty() {
        return String::new();
    }
    let Some(decimals) = decimals else {
        return amount.to_string();
    };
    if !amount.chars().all(|c| c.is_ascii_digit()) {
        return amount.to_string();
    }
    if decimals == 0 {
        return amount.to_string();
    }

    let d = decimals as usize;
    if amount.len() <= d {
        let mut frac = format!("{:0>width$}", amount, width = d);
        while frac.ends_with('0') {
            frac.pop();
        }
        return if frac.is_empty() {
            "0".to_string()
        } else {
            format!("0.{frac}")
        };
    }

    let (whole, frac) = amount.split_at(amount.len() - d);
    let mut frac = frac.to_string();
    while frac.ends_with('0') {
        frac.pop();
    }
    if frac.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{frac}")
    }
}

fn usd_estimate(intent: &EngineIntent) -> String {
    // Simple, transparent rule: we only compute USD when one side is a stablecoin.
    let net = intent.network.as_deref();

    if let (Some(buy), Some(min_buy)) = (
        intent.buy_token.as_deref(),
        intent.min_buy_amount.as_deref(),
    ) {
        if let Some(info) = token_info(net, buy) {
            if info.is_stable_usd {
                let v = format_amount_atoms(min_buy, Some(info.decimals));
                return if v.is_empty() {
                    String::new()
                } else {
                    format!("${v}")
                };
            }

            if let Some(rate) = info.usd_per_token {
                if let Some(qty) = parse_atoms_to_f64(min_buy, info.decimals) {
                    let usd = qty * rate;
                    return format!("${}", format_usd_number(usd, 2));
                }
            }
        }
    }

    if let (Some(sell), Some(sell_amt)) =
        (intent.sell_token.as_deref(), intent.sell_amount.as_deref())
    {
        if let Some(info) = token_info(net, sell) {
            if info.is_stable_usd {
                let v = format_amount_atoms(sell_amt, Some(info.decimals));
                return if v.is_empty() {
                    String::new()
                } else {
                    format!("${v}")
                };
            }

            if let Some(rate) = info.usd_per_token {
                if let Some(qty) = parse_atoms_to_f64(sell_amt, info.decimals) {
                    let usd = qty * rate;
                    return format!("${}", format_usd_number(usd, 2));
                }
            }
        }
    }

    String::new()
}

fn parse_atoms_to_f64(amount: &str, decimals: u8) -> Option<f64> {
    let s = amount.trim();
    if s.is_empty() {
        return None;
    }
    if !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let atoms: u128 = s.parse().ok()?;
    let denom = 10_f64.powi(decimals as i32);
    Some((atoms as f64) / denom)
}

fn format_usd_number(v: f64, decimals: usize) -> String {
    // Best-effort formatting; uses f64 so treat as approximate.
    let rounded = format!("{:.1$}", v, decimals);
    let (sign, rest) = if let Some(r) = rounded.strip_prefix('-') {
        ("-", r)
    } else {
        ("", rounded.as_str())
    };
    let mut parts = rest.split('.');
    let int_part = parts.next().unwrap_or("0");
    let frac_part = parts.next();

    let mut out_int = String::new();
    for (idx, ch) in int_part.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out_int.push(',');
        }
        out_int.push(ch);
    }
    let int_commas: String = out_int.chars().rev().collect();

    if let Some(frac) = frac_part {
        format!("{sign}{int_commas}.{frac}")
    } else {
        format!("{sign}{int_commas}")
    }
}

fn limit_price(intent: &EngineIntent) -> String {
    // If either side is a USD stablecoin, compute $ per *other* token unit.
    // - buy stable:  price = min_buy_usd / sell_amount  ($ per sell token)
    // - sell stable: price = sell_usd / min_buy_amount  ($ per buy token)
    let net = intent.network.as_deref();

    // Case 1: buy side is stable
    if let (Some(buy_addr), Some(min_buy_atoms), Some(sell_addr), Some(sell_atoms)) = (
        intent.buy_token.as_deref(),
        intent.min_buy_amount.as_deref(),
        intent.sell_token.as_deref(),
        intent.sell_amount.as_deref(),
    ) {
        if let (Some(buy_info), Some(sell_info)) =
            (token_info(net, buy_addr), token_info(net, sell_addr))
        {
            if buy_info.is_stable_usd {
                if let (Some(usd), Some(qty)) = (
                    parse_atoms_to_f64(min_buy_atoms, buy_info.decimals),
                    parse_atoms_to_f64(sell_atoms, sell_info.decimals),
                ) {
                    if qty > 0.0 {
                        let px = usd / qty;
                        return format!("${}/{}", format_usd_number(px, 2), sell_info.symbol);
                    }
                }
            } else if let Some(rate) = buy_info.usd_per_token {
                if let (Some(buy_qty), Some(qty)) = (
                    parse_atoms_to_f64(min_buy_atoms, buy_info.decimals),
                    parse_atoms_to_f64(sell_atoms, sell_info.decimals),
                ) {
                    if qty > 0.0 {
                        let usd = buy_qty * rate;
                        let px = usd / qty;
                        return format!("${}/{}", format_usd_number(px, 2), sell_info.symbol);
                    }
                }
            }
        }
    }

    // Case 2: sell side is stable
    if let (Some(sell_addr), Some(sell_atoms), Some(buy_addr), Some(min_buy_atoms)) = (
        intent.sell_token.as_deref(),
        intent.sell_amount.as_deref(),
        intent.buy_token.as_deref(),
        intent.min_buy_amount.as_deref(),
    ) {
        if let (Some(sell_info), Some(buy_info)) =
            (token_info(net, sell_addr), token_info(net, buy_addr))
        {
            if sell_info.is_stable_usd {
                if let (Some(usd), Some(qty)) = (
                    parse_atoms_to_f64(sell_atoms, sell_info.decimals),
                    parse_atoms_to_f64(min_buy_atoms, buy_info.decimals),
                ) {
                    if qty > 0.0 {
                        let px = usd / qty;
                        return format!("${}/{}", format_usd_number(px, 2), buy_info.symbol);
                    }
                }
            } else if let Some(rate) = sell_info.usd_per_token {
                if let (Some(sell_qty), Some(qty)) = (
                    parse_atoms_to_f64(sell_atoms, sell_info.decimals),
                    parse_atoms_to_f64(min_buy_atoms, buy_info.decimals),
                ) {
                    if qty > 0.0 {
                        let usd = sell_qty * rate;
                        let px = usd / qty;
                        return format!("${}/{}", format_usd_number(px, 2), buy_info.symbol);
                    }
                }
            }
        }
    }

    String::new()
}

fn buy_now_atoms(intent: &EngineIntent) -> Option<String> {
    // For UniswapX Dutch auctions, interpolate outputs[0] between start/end using decay times.
    // Fallback: use min_buy_amount.
    let start = intent.buy_amount_start.as_deref()?;
    let end = intent.buy_amount_end.as_deref()?;
    let ds = intent.decay_start_unix?;
    let de = intent.decay_end_unix?;
    if ds >= de {
        return Some(end.to_string());
    }

    let now = Utc::now().timestamp();
    if now <= ds {
        return Some(start.to_string());
    }
    if now >= de {
        return Some(end.to_string());
    }

    // Linear interpolation in integer space using u128.
    let s: u128 = start.parse().ok()?;
    let e: u128 = end.parse().ok()?;
    let t_num = (now - ds) as u128;
    let t_den = (de - ds) as u128;
    let cur = if s >= e {
        // amount decays down
        s - ((s - e) * t_num / t_den)
    } else {
        // amount decays up
        s + ((e - s) * t_num / t_den)
    };
    Some(cur.to_string())
}

fn buy_range_human(intent: &EngineIntent) -> String {
    let net = intent.network.as_deref();
    let buy_addr = intent.buy_token.as_deref().unwrap_or("");
    let buy_info = token_info(net, buy_addr);
    let start = intent.buy_amount_start.as_deref().unwrap_or("");
    let end = intent.buy_amount_end.as_deref().unwrap_or("");
    if start.is_empty() || end.is_empty() {
        return String::new();
    }
    let s = format_amount_atoms(start, buy_info.map(|t| t.decimals));
    let e = format_amount_atoms(end, buy_info.map(|t| t.decimals));
    format!("{s}→{e}")
}

fn decay_remaining(intent: &EngineIntent) -> String {
    let de = match intent.decay_end_unix {
        Some(v) => v,
        None => return String::new(),
    };
    let now = Utc::now().timestamp();
    let rem = (de - now).max(0);
    let m = rem / 60;
    let s = rem % 60;
    format!("{m:02}:{s:02}")
}

fn excl_flag(intent: &EngineIntent) -> String {
    let ex = match intent.exclusive_filler.as_deref() {
        Some(v) if !v.is_empty() => v,
        _ => return String::new(),
    };
    let until = match intent.exclusive_until_unix {
        Some(v) => v,
        None => return "EXCL".to_string(),
    };
    let now = Utc::now().timestamp();
    if now >= until {
        return String::new();
    }
    let rem = until - now;
    format!("EXCL {}s", rem)
}

#[derive(Debug, Deserialize, Clone)]
struct EngineState {
    #[allow(dead_code)]
    schema_version: String,
    emitted_at: DateTime<Utc>,
    active: Vec<EngineIntent>,
}

#[derive(Debug, Deserialize, Clone)]
struct EngineIntent {
    key: String,
    id: String,
    source: String,
    network: Option<String>,
    ttl_seconds: i64,
    expires_at: DateTime<Utc>,
    sell_token: Option<String>,
    buy_token: Option<String>,
    sell_amount: Option<String>,
    min_buy_amount: Option<String>,

    sell_amount_start: Option<String>,
    sell_amount_end: Option<String>,
    buy_amount_start: Option<String>,
    buy_amount_end: Option<String>,
    decay_start_unix: Option<i64>,
    decay_end_unix: Option<i64>,
    exclusive_filler: Option<String>,
    exclusive_until_unix: Option<i64>,
    order_type: Option<String>,
}

fn parse_source_filter_env() -> Option<HashSet<String>> {
    let v = std::env::var("TUI_SOURCES").ok()?;
    let mut set = HashSet::new();
    for part in v.split(',') {
        let s = part.trim().to_lowercase();
        if !s.is_empty() {
            set.insert(s);
        }
    }
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

fn source_allowed(filter: &Option<HashSet<String>>, source: &str) -> bool {
    match filter {
        None => true,
        Some(set) => set.contains(&source.to_lowercase()),
    }
}

fn main() -> Result<()> {
    let sub_endpoint =
        std::env::var("ENGINE_SUB").unwrap_or_else(|_| "tcp://engine:5556".to_string());

    // Optional initial filter: TUI_SOURCES=uniswapx,cowswap
    let mut source_filter = parse_source_filter_env();

    let (tx, rx) = mpsc::channel::<EngineState>();
    thread::spawn(move || {
        if let Err(err) = zmq_thread(sub_endpoint, tx) {
            eprintln!("tui: zmq thread error: {err:#}");
        }
    });

    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alt screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("terminal")?;

    let mut last_state: Option<EngineState> = None;
    let mut last_render = Instant::now();

    loop {
        while let Ok(st) = rx.try_recv() {
            last_state = Some(st);
        }

        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                if k.code == KeyCode::Char('q') {
                    break;
                }

                // Toggle common filters.
                // - 'u' => uniswapx only
                // - 'a' => all sources
                if k.code == KeyCode::Char('u') {
                    let mut set = HashSet::new();
                    set.insert("uniswapx".to_string());
                    source_filter = Some(set);
                }
                if k.code == KeyCode::Char('a') {
                    source_filter = None;
                }
            }
        }

        if last_render.elapsed() >= Duration::from_millis(250) {
            terminal
                .draw(|f| {
                    let area = f.area();
                    let chunks = Layout::vertical([Constraint::Min(0)]).split(area);

                    let header = Row::new([
                        Cell::from("Source"),
                        Cell::from("TTL"),
                        Cell::from("USD"),
                        Cell::from("LimitPx"),
                        Cell::from("Sell"),
                        Cell::from("Buy"),
                        Cell::from("SellAmt"),
                        Cell::from("BuyNow"),
                        Cell::from("BuyRange"),
                        Cell::from("Decay"),
                        Cell::from("Excl"),
                        Cell::from("ID"),
                    ])
                    .style(Style::default().fg(Color::Yellow));

                    let rows = match &last_state {
                        Some(st) => st
                            .active
                            .iter()
                            .filter(|i| source_allowed(&source_filter, &i.source))
                            .map(|i| {
                                let ttl = i.ttl_seconds.max(0).to_string();

                                let net = i.network.as_deref();
                                let sell_addr = i.sell_token.clone().unwrap_or_default();
                                let buy_addr = i.buy_token.clone().unwrap_or_default();
                                let sell_info = token_info(net, &sell_addr);
                                let buy_info = token_info(net, &buy_addr);

                                let sell_amt_raw = i.sell_amount.clone().unwrap_or_default();
                                let min_buy_raw = i.min_buy_amount.clone().unwrap_or_default();
                                let sell_amt = format_amount_atoms(
                                    &sell_amt_raw,
                                    sell_info.map(|t| t.decimals),
                                );

                                let buy_now_raw =
                                    buy_now_atoms(i).unwrap_or_else(|| min_buy_raw.clone());
                                let buy_now =
                                    format_amount_atoms(&buy_now_raw, buy_info.map(|t| t.decimals));
                                let buy_range = buy_range_human(i);
                                let decay = decay_remaining(i);
                                let excl = excl_flag(i);

                                Row::new([
                                    Cell::from(i.source.clone()),
                                    Cell::from(ttl),
                                    Cell::from(usd_estimate(i)),
                                    Cell::from(limit_price(i)),
                                    Cell::from(token_label(net, &sell_addr)),
                                    Cell::from(token_label(net, &buy_addr)),
                                    Cell::from(sell_amt),
                                    Cell::from(buy_now),
                                    Cell::from(buy_range),
                                    Cell::from(decay),
                                    Cell::from(excl),
                                    Cell::from(short_id(&i.id)),
                                ])
                            })
                            .collect::<Vec<_>>(),
                        None => vec![Row::new([Cell::from("waiting for engine...")])],
                    };

                    let widths = [
                        Constraint::Length(10),
                        Constraint::Length(5),
                        Constraint::Length(12),
                        Constraint::Length(16),
                        Constraint::Length(26),
                        Constraint::Length(26),
                        Constraint::Length(16),
                        Constraint::Length(14),
                        Constraint::Length(18),
                        Constraint::Length(6),
                        Constraint::Length(10),
                        Constraint::Min(12),
                    ];

                    let filter_label = match &source_filter {
                        None => "all".to_string(),
                        Some(s) if s.len() == 1 && s.contains("uniswapx") => "uniswapx".to_string(),
                        Some(s) => {
                            let mut v: Vec<String> = s.iter().cloned().collect();
                            v.sort();
                            v.join(",")
                        }
                    };

                    let title = match &last_state {
                        Some(st) => format!(
                            "Intent Market TUI  |  updated {}  |  filter={}  |  u=uniswap  a=all  q=quit",
                            st.emitted_at.to_rfc3339(),
                            filter_label
                        ),
                        None => {
                            "Intent Market TUI  |  connecting...  |  u=uniswap  a=all  q=quit".to_string()
                        }
                    };

                    let table = Table::new(rows, widths)
                        .header(header)
                        .block(Block::default().title(title).borders(Borders::ALL));

                    f.render_widget(table, chunks[0]);
                })
                .ok();

            last_render = Instant::now();
        }
    }

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    Ok(())
}

fn zmq_thread(sub_endpoint: String, tx: mpsc::Sender<EngineState>) -> Result<()> {
    let ctx = zmq::Context::new();
    let sub = ctx.socket(zmq::SUB).context("create SUB")?;
    sub.set_subscribe(b"").context("subscribe")?;
    sub.connect(&sub_endpoint)
        .with_context(|| format!("connect SUB: {sub_endpoint}"))?;

    loop {
        let msg = sub.recv_bytes(0).context("recv")?;
        if let Ok(st) = serde_json::from_slice::<EngineState>(&msg) {
            let _ = tx.send(st);
        }
    }
}

fn short_id(id: &str) -> String {
    if id.len() <= 16 {
        return id.to_string();
    }
    format!("{}…{}", &id[..8], &id[id.len() - 6..])
}
