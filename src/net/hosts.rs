/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2023 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::collections::HashSet;

use async_std::sync::{Arc, RwLock};
use log::debug;
use rand::{prelude::IteratorRandom, rngs::OsRng};
use url::Url;

use super::settings::SettingsPtr;

/// Atomic pointer to hosts object
pub type HostsPtr = Arc<Hosts>;

/// Manages a store of network addresses
pub struct Hosts {
    /// Set of stored addresses
    addrs: RwLock<HashSet<Url>>,
    /// Pointer to configured P2P settings
    settings: SettingsPtr,
}

impl Hosts {
    /// Create a new hosts list. Also initializes private IP ranges used
    /// for filtering.
    pub fn new(settings: SettingsPtr) -> HostsPtr {
        Arc::new(Self { addrs: RwLock::new(HashSet::new()), settings })
    }

    /// Append given addrs to the known set. Filtering should be done externally.
    pub async fn store(&self, addrs: &[Url]) {
        debug!(target: "net::hosts::store()", "hosts::store() [START]");

        let filtered_addrs = self.filter_addresses(addrs).await;

        if !filtered_addrs.is_empty() {
            let mut addrs_map = self.addrs.write().await;
            for addr in filtered_addrs {
                debug!(target: "net::hosts::store()", "Inserting {}", addr);
                addrs_map.insert(addr);
            }
        }

        debug!(target: "net::hosts::store()", "hosts::store() [END]");
    }

    /// Filter given addresses based on certain rulesets and validity.
    async fn filter_addresses(&self, addrs: &[Url]) -> Vec<Url> {
        let mut ret = vec![];
        let localnet = self.settings.localnet;

        for _addr in addrs {
            // Validate that the format is `scheme://host_str:port`
            if _addr.host_str().is_none() ||
                _addr.port().is_none() ||
                _addr.cannot_be_a_base() ||
                _addr.path_segments().is_some()
            {
                continue
            }

            let host_str = _addr.host_str().unwrap();

            if !localnet {
                // Our own addresses should never enter the hosts set.
                let mut got_own = false;
                for ext in &self.settings.external_addrs {
                    if host_str == ext.host_str().unwrap() {
                        got_own = true;
                        break
                    }
                }
                if got_own {
                    continue
                }
            }

            // We do this hack in order to parse IPs properly.
            // https://github.com/whatwg/url/issues/749
            let addr = Url::parse(&_addr.as_str().replace(_addr.scheme(), "http")).unwrap();

            // Filter non-global ranges if we're not allowing localnet.
            // Should never be allowed in production, so we don't really care
            // about some of them (e.g. 0.0.0.0, or broadcast, etc.).
            if !localnet {
                // Filter private IP ranges
                match addr.host().unwrap() {
                    url::Host::Ipv4(ip) => {
                        if !ip.is_global() {
                            continue
                        }
                    }
                    url::Host::Ipv6(ip) => {
                        if !ip.is_global() {
                            continue
                        }
                    }
                    url::Host::Domain(d) => {
                        // TODO: This could perhaps be more exhaustive?
                        if d == "localhost" {
                            continue
                        }
                    }
                }
            }

            match _addr.scheme() {
                // Validate that the address is an actual onion.
                #[cfg(feature = "p2p-transport-tor")]
                "tor" | "tor+tls" => {
                    use std::str::FromStr;
                    if tor_hscrypto::pk::HsId::from_str(host_str).is_err() {
                        continue
                    }
                }

                #[cfg(feature = "p2p-transport-nym")]
                "nym" | "nym+tls" => continue, // <-- Temp skip

                #[cfg(feature = "p2p-transport-tcp")]
                "tcp" | "tcp+tls" => {}

                _ => continue,
            }

            ret.push(_addr.clone());
        }

        ret
    }

    pub async fn remove(&self, url: &Url) -> bool {
        self.addrs.write().await.remove(url)
    }

    /// Check if the host list is empty.
    pub async fn is_empty(&self) -> bool {
        self.addrs.read().await.is_empty()
    }

    /// Check if host is already in the set
    pub async fn contains(&self, addr: &Url) -> bool {
        self.addrs.read().await.contains(addr)
    }

    /// Return all known hosts
    pub async fn load_all(&self) -> Vec<Url> {
        self.addrs.read().await.iter().cloned().collect()
    }

    /// Get up to n random hosts from the hosts set.
    pub async fn get_n_random(&self, n: u32) -> Vec<Url> {
        let n = n as usize;
        let addrs = self.addrs.read().await;
        let urls = addrs.iter().choose_multiple(&mut OsRng, n.min(addrs.len()));
        let urls = urls.iter().map(|&url| url.clone()).collect();
        urls
    }

    /// Get all peers that match the given transport schemes from the hosts set.
    pub async fn load_with_schemes(&self, schemes: &[String]) -> Vec<Url> {
        let mut ret = vec![];

        for addr in self.addrs.read().await.iter() {
            if schemes.contains(&addr.scheme().to_string()) {
                ret.push(addr.clone());
            }
        }

        ret
    }
}

#[cfg(test)]
mod tests {
    use super::{super::settings::Settings, *};

    #[async_std::test]
    async fn test_store_localnet() {
        let mut settings = Settings::default();
        settings.localnet = true;
        settings.external_addrs = vec![
            Url::parse("tcp://foo.bar:123").unwrap(),
            Url::parse("tcp://lol.cat:321").unwrap(),
        ];

        let hosts = Hosts::new(Arc::new(settings.clone()));
        hosts.store(&settings.external_addrs).await;
        assert!(hosts.is_empty().await);

        let local_hosts = vec![
            Url::parse("tcp://localhost:3921").unwrap(),
            Url::parse("tcp://127.0.0.1:23957").unwrap(),
            Url::parse("tcp://[::1]:21481").unwrap(),
            Url::parse("tcp://192.168.10.65:311").unwrap(),
            Url::parse("tcp://0.0.0.0:2312").unwrap(),
            Url::parse("tcp://255.255.255.255:2131").unwrap(),
        ];
        hosts.store(&local_hosts).await;
        assert!(hosts.is_empty().await);

        let remote_hosts = vec![
            Url::parse("tcp://dark.fi:80").unwrap(),
            Url::parse("tcp://top.kek:111").unwrap(),
            Url::parse("tcp://http.cat:401").unwrap(),
        ];
        hosts.store(&remote_hosts).await;
        assert!(hosts.is_empty().await);
    }

    #[async_std::test]
    async fn test_store() {
        let mut settings = Settings::default();
        settings.localnet = false;
        settings.external_addrs = vec![
            Url::parse("tcp://foo.bar:123").unwrap(),
            Url::parse("tcp://lol.cat:321").unwrap(),
        ];

        let hosts = Hosts::new(Arc::new(settings.clone()));
        hosts.store(&settings.external_addrs).await;
        assert!(hosts.is_empty().await);

        let local_hosts = vec![
            Url::parse("tcp://localhost:3921").unwrap(),
            Url::parse("tcp://127.0.0.1:23957").unwrap(),
            Url::parse("tor://[::1]:21481").unwrap(),
            Url::parse("tcp://192.168.10.65:311").unwrap(),
            Url::parse("tcp+tls://0.0.0.0:2312").unwrap(),
            Url::parse("tcp://255.255.255.255:2131").unwrap(),
        ];
        hosts.store(&local_hosts).await;
        assert!(hosts.is_empty().await);

        let remote_hosts = vec![
            Url::parse("tcp://dark.fi:80").unwrap(),
            Url::parse("tcp://http.cat:401").unwrap(),
            Url::parse("tcp://foo.bar:111").unwrap(),
        ];
        hosts.store(&remote_hosts).await;
        assert!(hosts.is_empty().await);
    }
}
