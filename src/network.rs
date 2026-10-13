//! Network screen: Ethernet and Wi-Fi status, Wi-Fi scanning and joining through
//! wpa_supplicant's control socket, and the Ethernet address (DHCP or static)
//! written as a dhcpcd configuration.
//!
//! Changing anything requires KLIPPER_HELM_MANAGE_NETWORK=1 (set by the embedded image's
//! start script), so running the panel on a PC only shows the status and never touches
//! the host's network setup.

use crate::config::{Config, NetworkConfig};
use crate::{AppWindow, Network, WifiNetwork};
use slint::{ComponentHandle, ModelRc, VecModel, Weak};
use std::cell::RefCell;
use std::io;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const WPA_CTRL_DIR: &str = "/var/run/wpa_supplicant";
const WPA_GLOBAL: &str = "/var/run/wpa_supplicant-global";
const DEFAULT_WPA_CONF: &str = "/data/network/wpa_supplicant.conf";
const DEFAULT_DHCPCD_CONF: &str = "/data/network/dhcpcd.conf";
const REFRESH: Duration = Duration::from_secs(3);
/// Scan results arrive a few seconds after a scan request
const SCAN_TIME: Duration = Duration::from_secs(6);

enum NetCommand {
    Scan,
    /// `hidden`: the network does not broadcast its name, so probe for it by name
    Connect { ssid: String, password: String, hidden: bool },
    Forget(String),
    ApplyEthernet(NetworkConfig),
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

pub fn wire(ui: &AppWindow, cfg: Rc<RefCell<Config>>) {
    let managed = std::env::var("KLIPPER_HELM_MANAGE_NETWORK").is_ok_and(|v| v == "1");
    let dhcpcd_conf: PathBuf = std::env::var_os("KLIPPER_HELM_DHCPCD_CONF")
        .map_or_else(|| DEFAULT_DHCPCD_CONF.into(), PathBuf::from);

    let (tx, rx) = mpsc::channel::<NetCommand>();
    let weak = ui.as_weak();
    std::thread::spawn(move || worker(weak, rx, managed, dhcpcd_conf));

    let net = ui.global::<Network>();
    net.set_managed(managed);
    net.set_networks(ModelRc::new(VecModel::<WifiNetwork>::default()));

    // Fill the Ethernet draft from the saved settings and look for networks
    let weak = ui.as_weak();
    let current = cfg.clone();
    let scan_tx = tx.clone();
    net.on_load(move || {
        let Some(ui) = weak.upgrade() else { return };
        let n = ui.global::<Network>();
        let c = &current.borrow().network;
        n.set_eth_static(c.eth_static);
        n.set_eth_address(c.eth_address.as_str().into());
        n.set_eth_gateway(c.eth_gateway.as_str().into());
        n.set_eth_dns(c.eth_dns.as_str().into());
        set_error(&n, "", "");
        let _ = scan_tx.send(NetCommand::Scan);
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
        let new = NetworkConfig {
            eth_static: n.get_eth_static(),
            eth_address: n.get_eth_address().trim().to_string(),
            eth_gateway: n.get_eth_gateway().trim().to_string(),
            eth_dns: n.get_eth_dns().trim().to_string(),
        };
        if let Err(e) = new.validate() {
            set_error(&n, e.code, &e.arg);
            return;
        }
        let mut updated = cfg.borrow().clone();
        updated.network = new.clone();
        if let Err(e) = updated.save() {
            set_error(&n, "save", &e.to_string());
            return;
        }
        *cfg.borrow_mut() = updated;
        set_error(&n, "", "");
        let _ = tx.send(NetCommand::ApplyEthernet(new));
    });
}

fn set_error(n: &Network<'_>, code: &str, arg: &str) {
    n.set_error_code(code.into());
    n.set_error_arg(arg.into());
}

fn worker(ui: Weak<AppWindow>, rx: Receiver<NetCommand>, managed: bool, dhcpcd_conf: PathBuf) {
    let mut scan_started: Option<Instant> = None;
    loop {
        match rx.recv_timeout(REFRESH) {
            Ok(cmd) => {
                if let Err(e) = handle(cmd, managed, &dhcpcd_conf, &mut scan_started) {
                    eprintln!("Network: {e}");
                    let message = e.to_string();
                    let _ = ui.upgrade_in_event_loop(move |ui| {
                        let n = ui.global::<Network>();
                        set_error(&n, "apply", &message);
                    });
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let scanning = scan_started.is_some_and(|t| t.elapsed() < SCAN_TIME);
        let status = read_status(scanning, managed);
        let _ = ui.upgrade_in_event_loop(move |ui| push(&ui, status));
    }
}

fn handle(
    cmd: NetCommand,
    managed: bool,
    dhcpcd_conf: &Path,
    scan_started: &mut Option<Instant>,
) -> io::Result<()> {
    // Scanning only reads, so it also works on a PC with a running wpa_supplicant
    if let NetCommand::Scan = cmd {
        if let Some(iface) = wifi_interface()
            && let Ok(wpa) = Wpa::open(&iface)
        {
            *scan_started = Some(Instant::now());
            // FAIL-BUSY just means a scan is already running
            let _ = wpa.request("SCAN");
        }
        return Ok(());
    }
    if !managed {
        return Err(io::Error::other("network changes are disabled on this system"));
    }
    match cmd {
        NetCommand::Scan => Ok(()),
        NetCommand::Connect { ssid, password, hidden } => {
            let iface = wifi_interface().ok_or_else(|| io::Error::other("no Wi-Fi adapter"))?;
            Wpa::managed(&iface)?.connect(&ssid, &password, hidden)
        }
        NetCommand::Forget(ssid) => {
            let iface = wifi_interface().ok_or_else(|| io::Error::other("no Wi-Fi adapter"))?;
            Wpa::managed(&iface)?.forget(&ssid)
        }
        NetCommand::ApplyEthernet(net) => {
            if let Some(dir) = dhcpcd_conf.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = dhcpcd_conf.with_extension("conf.tmp");
            std::fs::write(&tmp, dhcpcd_config(&net, ethernet_interface().as_deref()))?;
            std::fs::rename(&tmp, dhcpcd_conf)?;
            // Make the running dhcpcd re-read its configuration and renew the address
            let iface = ethernet_interface().unwrap_or_else(|| "eth0".into());
            let status = Command::new("dhcpcd").args(["--rebind", &iface]).status()?;
            if !status.success() {
                return Err(io::Error::other(format!("dhcpcd --rebind {iface} failed")));
            }
            Ok(())
        }
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

fn read_status(scanning: bool, managed: bool) -> NetStatus {
    let mut s = NetStatus { scanning, ..NetStatus::default() };
    if let Some(eth) = ethernet_interface() {
        s.eth_up = link_up(&eth);
        s.eth_ip = ipv4_address(&eth);
        s.eth_name = eth;
    }
    let Some(wlan) = wifi_interface() else { return s };
    s.wifi_present = true;
    s.wifi_ip = ipv4_address(&wlan);
    s.wifi_state = "disconnected".into();
    let wpa = match Wpa::open(&wlan) {
        Ok(wpa) => wpa,
        // Adapter present but not handed to wpa_supplicant (e.g. plugged in before it started):
        // attach it now, the next refresh picks it up
        Err(_) => {
            if managed {
                let _ = Wpa::attach(&wlan);
            }
            s.wifi_state = "unavailable".into();
            return s;
        }
    };
    if let Ok(status) = wpa.request("STATUS") {
        let field = |k: &str| {
            status.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(str::to_string)
        };
        s.wifi_ssid = field("ssid").map(|v| unescape(&v)).unwrap_or_default();
        s.wifi_state = match field("wpa_state").as_deref() {
            Some("COMPLETED") => "connected",
            Some("ASSOCIATING" | "ASSOCIATED" | "AUTHENTICATING" | "4WAY_HANDSHAKE" | "GROUP_HANDSHAKE") => {
                "connecting"
            }
            _ => "disconnected",
        }
        .into();
    }
    let saved = wpa.request("LIST_NETWORKS").map(|r| list_networks(&r)).unwrap_or_default();
    if let Ok(results) = wpa.request("SCAN_RESULTS") {
        let current = (s.wifi_state == "connected").then_some(s.wifi_ssid.as_str());
        s.networks = scan_results(&results, current, &saved);
    }
    s
}

/// SCAN_RESULTS lines: bssid, frequency, signal level (dBm), flags, ssid (tab-separated).
/// One entry per SSID (the strongest), the connected network first, then by signal.
fn scan_results(text: &str, current: Option<&str>, saved: &[(u32, String)]) -> Vec<WifiNetwork> {
    let mut best: Vec<(i32, WifiNetwork)> = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [_, _, level, flags, ssid] = cols[..] else { continue };
        let ssid = unescape(ssid);
        if ssid.is_empty() {
            continue; // hidden network
        }
        let dbm: i32 = level.parse().unwrap_or(-100);
        if let Some((d, _)) = best.iter().find(|(_, n)| n.ssid == ssid.as_str())
            && *d >= dbm
        {
            continue;
        }
        best.retain(|(_, n)| n.ssid != ssid.as_str());
        best.push((
            dbm,
            WifiNetwork {
                current: current == Some(ssid.as_str()),
                saved: saved.iter().any(|(_, s)| *s == ssid),
                secure: ["WPA", "RSN", "WEP", "SAE"].iter().any(|f| flags.contains(f)),
                signal: signal_strength(dbm),
                ssid: ssid.into(),
            },
        ));
    }
    best.sort_by(|a, b| b.1.current.cmp(&a.1.current).then(b.0.cmp(&a.0)));
    best.into_iter().take(30).map(|(_, n)| n).collect()
}

/// -90 dBm (unusable) .. -50 dBm (excellent) mapped to 0..1
fn signal_strength(dbm: i32) -> f32 {
    ((dbm + 90) as f32 / 40.0).clamp(0.05, 1.0)
}

/// LIST_NETWORKS lines: network id, ssid, bssid, flags
fn list_networks(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let mut cols = l.split('\t');
            let id = cols.next()?.parse().ok()?;
            Some((id, unescape(cols.next()?)))
        })
        .collect()
}

/// wpa_supplicant prints SSIDs with \\, \" and \xNN escapes
fn unescape(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let raw = s.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\' && i + 1 < raw.len() {
            match raw[i + 1] {
                b'x' if i + 3 < raw.len() => {
                    if let Ok(b) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                        bytes.push(b);
                        i += 4;
                        continue;
                    }
                }
                c @ (b'\\' | b'"') => {
                    bytes.push(c);
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

// --- wpa_supplicant control socket ------------------------------------------

struct Wpa {
    sock: UnixDatagram,
    local: PathBuf,
}

impl Wpa {
    /// Control connection for an interface, attaching it to wpa_supplicant first if needed.
    fn managed(iface: &str) -> io::Result<Self> {
        if let Ok(wpa) = Wpa::open(iface) {
            return Ok(wpa);
        }
        Wpa::attach(iface)?;
        Wpa::open(iface).map_err(|e| io::Error::other(format!("wpa_supplicant does not manage {iface} ({e})")))
    }

    /// Hands an interface to the running wpa_supplicant through its global control socket.
    fn attach(iface: &str) -> io::Result<()> {
        if !Path::new(WPA_GLOBAL).exists() {
            return Err(io::Error::other("wpa_supplicant is not running"));
        }
        let conf = std::env::var("KLIPPER_HELM_WPA_CONF").unwrap_or_else(|_| DEFAULT_WPA_CONF.into());
        let global = Wpa::connect_to(Path::new(WPA_GLOBAL))?;
        let reply = global.request(&format!("INTERFACE_ADD {iface}\t{conf}\tnl80211\t{WPA_CTRL_DIR}"))?;
        match reply.trim() {
            "OK" => Ok(()),
            other => Err(io::Error::other(format!("wpa_supplicant could not take over {iface}: {other}"))),
        }
    }

    fn open(iface: &str) -> io::Result<Self> {
        Wpa::connect_to(&Path::new(WPA_CTRL_DIR).join(iface))
    }

    fn connect_to(socket: &Path) -> io::Result<Self> {
        let local = std::env::temp_dir().join(format!("klipper-helm-wpa-{}", std::process::id()));
        let _ = std::fs::remove_file(&local);
        let sock = UnixDatagram::bind(&local)?;
        let wpa = Wpa { sock, local };
        wpa.sock.connect(socket)?;
        wpa.sock.set_read_timeout(Some(Duration::from_secs(3)))?;
        Ok(wpa)
    }

    fn request(&self, cmd: &str) -> io::Result<String> {
        self.sock.send(cmd.as_bytes())?;
        let mut buf = vec![0u8; 16384];
        loop {
            let n = self.sock.recv(&mut buf)?;
            let reply = String::from_utf8_lossy(&buf[..n]).into_owned();
            // Unsolicited event messages start with "<level>"; we never attach, but skip them anyway
            if !reply.starts_with('<') {
                return Ok(reply);
            }
        }
    }

    fn ok(&self, cmd: &str) -> io::Result<()> {
        let reply = self.request(cmd)?;
        if reply.trim() == "OK" {
            Ok(())
        } else {
            Err(io::Error::other(format!("wpa_supplicant: {} -> {}", cmd.split(' ').next().unwrap_or(cmd), reply.trim())))
        }
    }

    fn connect(&self, ssid: &str, password: &str, hidden: bool) -> io::Result<()> {
        let saved = list_networks(&self.request("LIST_NETWORKS")?);
        // No password for a saved network: rejoin it with what is stored
        if password.is_empty()
            && let Some((id, _)) = saved.iter().find(|(_, s)| s == ssid)
        {
            self.ok(&format!("SELECT_NETWORK {id}"))?;
            return self.ok("SAVE_CONFIG");
        }
        let id = match saved.iter().find(|(_, s)| s == ssid) {
            Some((id, _)) => *id,
            None => self.request("ADD_NETWORK")?.trim().parse::<u32>().map_err(io::Error::other)?,
        };
        // The SSID as hex needs no quoting, whatever characters it contains
        self.ok(&format!("SET_NETWORK {id} ssid {}", hex(ssid)))?;
        if hidden {
            self.ok(&format!("SET_NETWORK {id} scan_ssid 1"))?;
        }
        if password.is_empty() {
            self.ok(&format!("SET_NETWORK {id} key_mgmt NONE"))?;
        } else {
            // WPA2 and WPA3 personal; wpa_supplicant takes everything between the first and last quote
            self.ok(&format!("SET_NETWORK {id} key_mgmt WPA-PSK SAE"))?;
            self.ok(&format!("SET_NETWORK {id} ieee80211w 1"))?;
            self.ok(&format!("SET_NETWORK {id} psk \"{password}\""))?;
        }
        self.ok(&format!("SELECT_NETWORK {id}"))?;
        self.ok("SAVE_CONFIG")
    }

    fn forget(&self, ssid: &str) -> io::Result<()> {
        for (id, _) in list_networks(&self.request("LIST_NETWORKS")?).iter().filter(|(_, s)| s == ssid) {
            self.ok(&format!("REMOVE_NETWORK {id}"))?;
        }
        self.ok("SAVE_CONFIG")
    }
}

impl Drop for Wpa {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.local);
    }
}

// --- dhcpcd -----------------------------------------------------------------

/// dhcpcd configuration: DHCP everywhere, plus a static address for Ethernet if chosen.
fn dhcpcd_config(net: &NetworkConfig, eth: Option<&str>) -> String {
    let mut conf = String::from(
        "# Written by Klipper Helm (System -> Network); changes made here are overwritten.\n\
         hostname\n\
         clientid\n\
         persistent\n\
         option rapid_commit\n\
         option domain_name_servers, domain_name, domain_search, host_name\n\
         option classless_static_routes\n\
         option interface_mtu\n\
         require dhcp_server_identifier\n\
         slaac private\n\
         denyinterfaces lo\n",
    );
    if net.eth_static {
        let address = if net.eth_address.contains('/') {
            net.eth_address.clone()
        } else {
            format!("{}/24", net.eth_address)
        };
        conf += &format!("\ninterface {}\n", eth.unwrap_or("eth0"));
        conf += &format!("static ip_address={address}\n");
        if !net.eth_gateway.is_empty() {
            conf += &format!("static routers={}\n", net.eth_gateway);
        }
        let dns: Vec<&str> = net.eth_dns.split([' ', ',']).filter(|s| !s.is_empty()).collect();
        if !dns.is_empty() {
            conf += &format!("static domain_name_servers={}\n", dns.join(" "));
        }
    }
    conf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ip_output() {
        let out = "2: eth0    inet 192.168.1.23/24 brd 192.168.1.255 scope global eth0\\       valid_lft forever\n";
        assert_eq!(parse_ip_addr(out), "192.168.1.23");
        assert_eq!(parse_ip_addr(""), "");
    }

    #[test]
    fn unescapes_ssids() {
        assert_eq!(unescape(r#"Caf\xc3\xa9 \"1\""#), "Café \"1\"");
        assert_eq!(unescape(r"a\\b"), r"a\b");
    }

    #[test]
    fn merges_scan_results() {
        let text = "bssid / frequency / signal level / flags / ssid\n\
            aa:aa\t2412\t-70\t[WPA2-PSK-CCMP][ESS]\tHome\n\
            bb:bb\t5180\t-50\t[WPA2-PSK-CCMP][ESS]\tHome\n\
            cc:cc\t2437\t-60\t[ESS]\tCafe\n\
            dd:dd\t2437\t-40\t[ESS]\t\n";
        let list = scan_results(text, Some("Cafe"), &[(0, "Home".into())]);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].ssid, "Cafe"); // connected first
        assert!(!list[0].secure && list[0].current);
        assert!(list[1].secure && list[1].saved);
        assert_eq!(list[1].signal, signal_strength(-50)); // strongest of the two
    }

    #[test]
    fn writes_static_ethernet() {
        let net = NetworkConfig {
            eth_static: true,
            eth_address: "192.168.1.20".into(),
            eth_gateway: "192.168.1.1".into(),
            eth_dns: "1.1.1.1, 8.8.8.8".into(),
        };
        let conf = dhcpcd_config(&net, Some("eth0"));
        assert!(conf.contains("interface eth0\nstatic ip_address=192.168.1.20/24\n"));
        assert!(conf.contains("static routers=192.168.1.1\n"));
        assert!(conf.contains("static domain_name_servers=1.1.1.1 8.8.8.8\n"));
        assert!(!dhcpcd_config(&NetworkConfig::default(), None).contains("static ip_address"));
    }
}
