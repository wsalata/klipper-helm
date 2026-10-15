//! Network screen through NetworkManager (`nmcli`), as used by Raspberry Pi OS: Ethernet
//! and Wi-Fi status, scanning and joining Wi-Fi networks (hidden ones too), and the
//! Ethernet address (DHCP or static).
//!
//! Changing anything requires KLIPPER_HELM_MANAGE_NETWORK=1, which install.sh sets together
//! with a polkit rule that lets the panel's user change NetworkManager's settings. Without
//! it (e.g. running the panel on a PC) the screen only shows the status.

use crate::config::ConfigError;
use crate::{AppWindow, Network, WifiNetwork};
use slint::{ComponentHandle, ModelRc, VecModel, Weak};
use std::io;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(3);
/// NetworkManager updates its scan results a few seconds after a rescan request
const SCAN_TIME: Duration = Duration::from_secs(6);
/// How long nmcli waits for a connection to come up
const CONNECT_WAIT: &str = "30";

/// Ethernet address settings as edited on the screen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EthSettings {
    /// false: DHCP
    pub static_ip: bool,
    /// "192.168.1.20/24" (a missing prefix means /24)
    pub address: String,
    pub gateway: String,
    /// One or more servers separated by spaces or commas
    pub dns: String,
}

impl EthSettings {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.static_ip {
            return Ok(());
        }
        let (ip, prefix) = self.address.split_once('/').unwrap_or((&self.address, "24"));
        if ip.parse::<std::net::Ipv4Addr>().is_err() || !prefix.parse::<u8>().is_ok_and(|p| (1..=32).contains(&p)) {
            return Err(ConfigError::new("ip-address", self.address.clone()));
        }
        if !self.gateway.is_empty() && self.gateway.parse::<std::net::Ipv4Addr>().is_err() {
            return Err(ConfigError::new("ip-gateway", self.gateway.clone()));
        }
        for dns in self.dns_servers() {
            if dns.parse::<std::net::Ipv4Addr>().is_err() {
                return Err(ConfigError::new("ip-dns", dns));
            }
        }
        Ok(())
    }

    fn dns_servers(&self) -> impl Iterator<Item = &str> {
        self.dns.split([' ', ',']).filter(|s| !s.is_empty())
    }

    /// `nmcli connection modify` arguments for the IPv4 settings.
    fn nmcli_args(&self) -> Vec<String> {
        if !self.static_ip {
            return ["ipv4.method", "auto", "ipv4.addresses", "", "ipv4.gateway", "", "ipv4.dns", ""]
                .map(String::from)
                .to_vec();
        }
        let address = if self.address.contains('/') { self.address.clone() } else { format!("{}/24", self.address) };
        let dns: Vec<&str> = self.dns_servers().collect();
        vec![
            "ipv4.method".into(),
            "manual".into(),
            "ipv4.addresses".into(),
            address,
            "ipv4.gateway".into(),
            self.gateway.clone(),
            "ipv4.dns".into(),
            dns.join(","),
        ]
    }

    /// From `nmcli -t -f ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns connection show`.
    fn from_nmcli(fields: &[(String, String)]) -> Self {
        let get = |k: &str| {
            fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.trim()).filter(|v| *v != "--").unwrap_or("")
        };
        Self {
            static_ip: get("ipv4.method") == "manual",
            address: get("ipv4.addresses").split(',').next().unwrap_or("").trim().to_string(),
            gateway: get("ipv4.gateway").to_string(),
            dns: get("ipv4.dns").replace(',', " "),
        }
    }
}

enum NetCommand {
    /// Read the Ethernet settings into the screen's draft and look for networks
    Load,
    Scan,
    /// `hidden`: the network does not broadcast its name, so probe for it by name
    Connect { ssid: String, password: String, hidden: bool },
    Forget(String),
    ApplyEthernet(EthSettings),
}

/// Status sent to the UI after every refresh.
#[derive(Default)]
struct NetStatus {
    eth_name: String,
    eth_up: bool,
    eth_ip: String,
    wifi_present: bool,
    wifi_state: String,
    wifi_ssid: String,
    wifi_ip: String,
    scanning: bool,
    networks: Vec<WifiNetwork>,
}

pub fn wire(ui: &AppWindow) {
    let managed = std::env::var("KLIPPER_HELM_MANAGE_NETWORK").is_ok_and(|v| v == "1");

    let (tx, rx) = mpsc::channel::<NetCommand>();
    let weak = ui.as_weak();
    std::thread::spawn(move || worker(weak, rx, managed));

    let net = ui.global::<Network>();
    net.set_managed(managed);
    net.set_networks(ModelRc::new(VecModel::<WifiNetwork>::default()));

    let load_tx = tx.clone();
    let weak = ui.as_weak();
    net.on_load(move || {
        if let Some(ui) = weak.upgrade() {
            set_error(&ui.global::<Network>(), "", "");
        }
        let _ = load_tx.send(NetCommand::Load);
    });

    let weak = ui.as_weak();
    net.on_edit_text(move |id, value| {
        let Some(ui) = weak.upgrade() else { return };
        let n = ui.global::<Network>();
        let value = value.trim();
        match id.as_str() {
            "eth-address" => n.set_eth_address(value.into()),
            "eth-gateway" => n.set_eth_gateway(value.into()),
            "eth-dns" => n.set_eth_dns(value.into()),
            _ => {}
        }
    });

    let scan_tx = tx.clone();
    net.on_scan(move || {
        let _ = scan_tx.send(NetCommand::Scan);
    });

    let connect_tx = tx.clone();
    net.on_connect(move |ssid, password| {
        let _ = connect_tx.send(NetCommand::Connect { ssid: ssid.into(), password: password.into(), hidden: false });
    });

    let hidden_tx = tx.clone();
    net.on_connect_hidden(move |ssid, password| {
        let ssid = ssid.trim();
        if !ssid.is_empty() {
            let _ = hidden_tx.send(NetCommand::Connect { ssid: ssid.into(), password: password.into(), hidden: true });
        }
    });

    let forget_tx = tx.clone();
    net.on_forget(move |ssid| {
        let _ = forget_tx.send(NetCommand::Forget(ssid.into()));
    });

    let weak = ui.as_weak();
    net.on_apply_ethernet(move || {
        let Some(ui) = weak.upgrade() else { return };
        let n = ui.global::<Network>();
        let settings = EthSettings {
            static_ip: n.get_eth_static(),
            address: n.get_eth_address().trim().to_string(),
            gateway: n.get_eth_gateway().trim().to_string(),
            dns: n.get_eth_dns().trim().to_string(),
        };
        if let Err(e) = settings.validate() {
            set_error(&n, e.code, &e.arg);
            return;
        }
        set_error(&n, "", "");
        let _ = tx.send(NetCommand::ApplyEthernet(settings));
    });
}

fn set_error(n: &Network<'_>, code: &str, arg: &str) {
    n.set_error_code(code.into());
    n.set_error_arg(arg.into());
}

fn worker(ui: Weak<AppWindow>, rx: Receiver<NetCommand>, managed: bool) {
    let mut scan_started: Option<Instant> = None;
    // SSID -> NetworkManager connection name, refreshed when the screen opens and after changes
    let mut saved = saved_wifi();
    loop {
        match rx.recv_timeout(REFRESH) {
            Ok(cmd) => {
                let changes = !matches!(cmd, NetCommand::Load | NetCommand::Scan);
                match handle(cmd, managed, &saved, &mut scan_started, &ui) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Network: {e}");
                        let message = e.to_string();
                        let _ = ui.upgrade_in_event_loop(move |ui| {
                            set_error(&ui.global::<Network>(), "apply", &message);
                        });
                    }
                }
                if changes || saved.is_empty() {
                    saved = saved_wifi();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let scanning = scan_started.is_some_and(|t| t.elapsed() < SCAN_TIME);
        let status = read_status(scanning, &saved);
        let _ = ui.upgrade_in_event_loop(move |ui| push(&ui, status));
    }
}

fn handle(
    cmd: NetCommand,
    managed: bool,
    saved: &[(String, String)],
    scan_started: &mut Option<Instant>,
    ui: &Weak<AppWindow>,
) -> io::Result<()> {
    match cmd {
        // Reading and scanning also work without permission to change anything
        NetCommand::Load => {
            let settings = ethernet_interface()
                .and_then(|eth| ethernet_connection(&eth))
                .map(|conn| {
                    let fields = nmcli_fields(&[
                        "-f",
                        "ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns",
                        "connection",
                        "show",
                        "id",
                        &conn,
                    ]);
                    EthSettings::from_nmcli(&fields)
                })
                .unwrap_or_default();
            let _ = ui.upgrade_in_event_loop(move |ui| {
                let n = ui.global::<Network>();
                n.set_eth_static(settings.static_ip);
                n.set_eth_address(settings.address.into());
                n.set_eth_gateway(settings.gateway.into());
                n.set_eth_dns(settings.dns.into());
            });
            scan(scan_started);
            Ok(())
        }
        NetCommand::Scan => {
            scan(scan_started);
            Ok(())
        }
        _ if !managed => Err(io::Error::other("network changes are disabled on this system")),
        NetCommand::Connect { ssid, password, hidden } => {
            let wlan = wifi_interface().ok_or_else(|| io::Error::other("no Wi-Fi adapter"))?;
            let known: Vec<&str> = saved.iter().filter(|(s, _)| *s == ssid).map(|(_, c)| c.as_str()).collect();
            if password.is_empty() && !hidden && let Some(conn) = known.first() {
                return nmcli(&["-w", CONNECT_WAIT, "connection", "up", "id", conn]).map(drop);
            }
            // A new password replaces the saved connection instead of adding "<ssid> 1"
            for conn in known {
                nmcli(&["connection", "delete", "id", conn])?;
            }
            let mut args = vec!["-w", CONNECT_WAIT, "device", "wifi", "connect", &ssid, "ifname", &wlan];
            if !password.is_empty() {
                args.extend(["password", &password]);
            }
            if hidden {
                args.extend(["hidden", "yes"]);
            }
            nmcli(&args).map(drop)
        }
        NetCommand::Forget(ssid) => {
            for (_, conn) in saved.iter().filter(|(s, _)| *s == ssid) {
                nmcli(&["connection", "delete", "id", conn])?;
            }
            Ok(())
        }
        NetCommand::ApplyEthernet(settings) => {
            let eth = ethernet_interface().ok_or_else(|| io::Error::other("no Ethernet port"))?;
            let conn = match ethernet_connection(&eth) {
                Some(conn) => conn,
                None => {
                    let name = "Wired connection 1";
                    nmcli(&["connection", "add", "type", "ethernet", "ifname", &eth, "con-name", name])?;
                    name.to_string()
                }
            };
            let mut args: Vec<String> = ["connection", "modify", "id", &conn].map(String::from).to_vec();
            args.extend(settings.nmcli_args());
            nmcli(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            // Without a cable the settings are saved and used once one is plugged in
            if link_up(&eth) {
                nmcli(&["-w", CONNECT_WAIT, "connection", "up", "id", &conn])?;
            }
            Ok(())
        }
    }
}

fn scan(scan_started: &mut Option<Instant>) {
    if wifi_interface().is_some() {
        *scan_started = Some(Instant::now());
        // Refused for a few seconds after the previous scan; the old results stay valid
        let _ = nmcli(&["device", "wifi", "rescan"]);
    }
}

fn push(ui: &AppWindow, s: NetStatus) {
    let n = ui.global::<Network>();
    n.set_eth_name(s.eth_name.into());
    n.set_eth_up(s.eth_up);
    n.set_eth_ip(s.eth_ip.into());
    n.set_wifi_present(s.wifi_present);
    n.set_wifi_state(s.wifi_state.into());
    n.set_wifi_ssid(s.wifi_ssid.into());
    n.set_wifi_ip(s.wifi_ip.into());
    n.set_scanning(s.scanning);
    crate::bridge::sync(&n.get_networks(), s.networks);
}

// --- nmcli ------------------------------------------------------------------

/// Runs nmcli (in English, so states can be compared) and returns its output.
fn nmcli(args: &[&str]) -> io::Result<String> {
    let out = Command::new("nmcli").args(args).env("LC_ALL", "C").output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim().trim_start_matches("Error: ");
        Err(io::Error::other(if err.is_empty() { format!("nmcli {} failed", args.join(" ")) } else { err.to_string() }))
    }
}

/// Rows of `nmcli -t` table output.
fn nmcli_rows(args: &[&str]) -> Vec<Vec<String>> {
    let mut full = vec!["-t"];
    full.extend_from_slice(args);
    nmcli(&full).map(|out| out.lines().map(split_terse).collect()).unwrap_or_default()
}

/// `key:value` lines of `nmcli -t` detail output (`connection show <name>`, `device show`).
fn nmcli_fields(args: &[&str]) -> Vec<(String, String)> {
    let mut full = vec!["-t"];
    full.extend_from_slice(args);
    nmcli(&full)
        .map(|out| {
            out.lines()
                .filter_map(|l| l.split_once(':').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Splits a terse line at ':' separators; values escape ':' and '\' with a backslash.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields.last_mut().unwrap().push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// Saved Wi-Fi connections as (SSID, connection name). The name often differs from the SSID
/// (Raspberry Pi Imager calls its connection "preconfigured").
fn saved_wifi() -> Vec<(String, String)> {
    nmcli_rows(&["-f", "NAME,TYPE", "connection", "show"])
        .into_iter()
        .filter(|row| row.get(1).is_some_and(|t| t == "802-11-wireless"))
        .filter_map(|row| {
            let name = row.into_iter().next()?;
            let fields = nmcli_fields(&["-f", "802-11-wireless.ssid", "connection", "show", "id", &name]);
            let ssid = fields.into_iter().next()?.1;
            Some((ssid, name))
        })
        .collect()
}

/// The connection NetworkManager uses (or would use) for the Ethernet port.
fn ethernet_connection(eth: &str) -> Option<String> {
    let active = nmcli_fields(&["-f", "GENERAL.CONNECTION", "device", "show", eth])
        .into_iter()
        .map(|(_, v)| v)
        .find(|v| !v.is_empty() && v != "--");
    active.or_else(|| {
        nmcli_rows(&["-f", "NAME,TYPE,DEVICE", "connection", "show"])
            .into_iter()
            .find(|row| row.get(1).is_some_and(|t| t == "802-3-ethernet"))
            .and_then(|row| row.into_iter().next())
    })
}

// --- status -----------------------------------------------------------------

fn interfaces() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/net")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn is_wireless(iface: &str) -> bool {
    Path::new(&format!("/sys/class/net/{iface}/wireless")).exists()
}

/// A physical (not virtual) interface has a `device` link in sysfs
fn is_physical(iface: &str) -> bool {
    Path::new(&format!("/sys/class/net/{iface}/device")).exists()
}

fn ethernet_interface() -> Option<String> {
    interfaces().into_iter().find(|i| is_physical(i) && !is_wireless(i))
}

fn wifi_interface() -> Option<String> {
    interfaces().into_iter().find(|i| is_wireless(i))
}

fn link_up(iface: &str) -> bool {
    std::fs::read_to_string(format!("/sys/class/net/{iface}/operstate")).is_ok_and(|s| s.trim() == "up")
}

/// First IPv4 address of an interface, from `ip -4 -o addr show dev <iface>`
fn ipv4_address(iface: &str) -> String {
    let Ok(out) = Command::new("ip").args(["-4", "-o", "addr", "show", "dev", iface]).output() else {
        return String::new();
    };
    parse_ip_addr(&String::from_utf8_lossy(&out.stdout))
}

fn parse_ip_addr(output: &str) -> String {
    output
        .split_whitespace()
        .skip_while(|w| *w != "inet")
        .nth(1)
        .map(|a| a.split('/').next().unwrap_or(a).to_string())
        .unwrap_or_default()
}

fn read_status(scanning: bool, saved: &[(String, String)]) -> NetStatus {
    let mut s = NetStatus { scanning, ..NetStatus::default() };
    if let Some(eth) = ethernet_interface() {
        s.eth_up = link_up(&eth);
        s.eth_ip = ipv4_address(&eth);
        s.eth_name = eth;
    }
    let Some(wlan) = wifi_interface() else { return s };
    s.wifi_present = true;
    s.wifi_ip = ipv4_address(&wlan);

    let devices = nmcli_rows(&["-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"]);
    let Some(device) = devices.iter().find(|row| row.first() == Some(&wlan)) else {
        // NetworkManager is not running (or does not manage the adapter)
        s.wifi_state = "unavailable".into();
        return s;
    };
    let state = device.get(2).map_or("", String::as_str);
    s.wifi_state = if state == "connected" {
        "connected"
    } else if state.starts_with("connecting") {
        "connecting"
    } else if state == "unavailable" || state == "unmanaged" {
        "unavailable"
    } else {
        "disconnected"
    }
    .into();

    let list = nmcli_rows(&["-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", "no"]);
    s.networks = wifi_list(&list, saved);
    s.wifi_ssid = s
        .networks
        .iter()
        .find(|n| n.current)
        .map(|n| n.ssid.to_string())
        .or_else(|| device.get(3).cloned())
        .unwrap_or_default();
    s
}

/// `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list` rows. One entry per SSID
/// (the strongest), the connected network first, then by signal.
fn wifi_list(rows: &[Vec<String>], saved: &[(String, String)]) -> Vec<WifiNetwork> {
    let mut best: Vec<(i32, WifiNetwork)> = Vec::new();
    for row in rows {
        let [in_use, ssid, signal, security] = &row[..] else { continue };
        if ssid.is_empty() {
            continue; // hidden network
        }
        let signal: i32 = signal.parse().unwrap_or(0);
        let current = in_use == "*";
        if let Some((d, n)) = best.iter_mut().find(|(_, n)| n.ssid == ssid.as_str()) {
            n.current |= current;
            if *d >= signal {
                continue;
            }
            *d = signal;
            n.signal = signal as f32 / 100.0;
            continue;
        }
        best.push((
            signal,
            WifiNetwork {
                current,
                saved: saved.iter().any(|(s, _)| s == ssid),
                secure: !security.is_empty() && security != "--",
                signal: (signal as f32 / 100.0).clamp(0.05, 1.0),
                ssid: ssid.as_str().into(),
            },
        ));
    }
    best.sort_by(|a, b| b.1.current.cmp(&a.1.current).then(b.0.cmp(&a.0)));
    best.into_iter().take(30).map(|(_, n)| n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(fields: &[&str]) -> Vec<String> {
        fields.iter().map(|f| f.to_string()).collect()
    }

    #[test]
    fn parses_ip_output() {
        let out = "2: eth0    inet 192.168.1.20/24 brd 192.168.1.255 scope global eth0\\       valid_lft forever";
        assert_eq!(parse_ip_addr(out), "192.168.1.20");
        assert_eq!(parse_ip_addr(""), "");
    }

    #[test]
    fn splits_terse_lines() {
        assert_eq!(split_terse(r"*:My\:Net:80:WPA2"), row(&["*", "My:Net", "80", "WPA2"]));
        assert_eq!(split_terse(r":back\\slash::"), row(&["", r"back\slash", "", ""]));
    }

    #[test]
    fn merges_wifi_list() {
        let rows = [
            row(&["", "Home", "40", "WPA2"]),
            row(&["*", "Home", "70", "WPA2"]),
            row(&["", "Cafe", "90", ""]),
            row(&["", "", "60", "WPA2"]),
        ];
        let saved = [("Home".to_string(), "preconfigured".to_string())];
        let list = wifi_list(&rows, &saved);
        let names: Vec<_> = list.iter().map(|n| (n.ssid.to_string(), n.current, n.saved, n.secure)).collect();
        assert_eq!(names, [("Home".into(), true, true, true), ("Cafe".into(), false, false, false)]);
        assert_eq!(list[0].signal, 0.7);
    }

    #[test]
    fn ethernet_settings() {
        let mut eth = EthSettings { static_ip: true, address: "192.168.1.20".into(), ..Default::default() };
        assert!(eth.validate().is_ok());
        assert_eq!(eth.nmcli_args()[3], "192.168.1.20/24");
        eth.address = "192.168.1.20/33".into();
        assert_eq!(eth.validate().unwrap_err().code, "ip-address");
        eth.address = "10.0.0.5/8".into();
        eth.dns = "1.1.1.1, nope".into();
        assert_eq!(eth.validate().unwrap_err(), ConfigError::new("ip-dns", "nope"));
        eth.dns = "1.1.1.1 8.8.8.8".into();
        assert_eq!(eth.nmcli_args()[7], "1.1.1.1,8.8.8.8");
        // DHCP ignores the static fields and clears them in NetworkManager
        let dhcp = EthSettings { address: "x".into(), ..Default::default() };
        assert!(dhcp.validate().is_ok());
        assert_eq!(dhcp.nmcli_args()[..2], ["ipv4.method", "auto"]);

        let fields = [
            ("ipv4.method".to_string(), "manual".to_string()),
            ("ipv4.addresses".into(), "192.168.1.20/24".into()),
            ("ipv4.gateway".into(), "192.168.1.1".into()),
            ("ipv4.dns".into(), "1.1.1.1,8.8.8.8".into()),
        ];
        let read = EthSettings::from_nmcli(&fields);
        assert_eq!(read.address, "192.168.1.20/24");
        assert_eq!(read.dns, "1.1.1.1 8.8.8.8");
        assert!(read.static_ip);
        let auto = EthSettings::from_nmcli(&[("ipv4.gateway".into(), "--".into())]);
        assert_eq!(auto, EthSettings::default());
    }
}
