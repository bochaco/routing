// Copyright 2020 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use routing::{Error, EventStream, FullId, Node, NodeConfig, Result, TransportConfig};
use std::{
    collections::{BTreeSet, HashSet},
    io::Write,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Once,
};

static LOG_INIT: Once = Once::new();

// -----  TestNode and builder  -----

pub struct TestNodeBuilder {
    config: NodeConfig,
}

impl<'a> TestNodeBuilder {
    pub fn new(config: Option<NodeConfig>) -> Self {
        // We initialise the logger but only once for all tests
        LOG_INIT.call_once(|| {
            env_logger::builder()
                // the test framework will capture the log output and show it only on failure.
                // Run the tests with --nocapture to override.
                .is_test(true)
                .format(|buf, record| {
                    writeln!(
                        buf,
                        "{:.1} {} ({}:{})",
                        record.level(),
                        record.args(),
                        record.file().unwrap_or("<unknown>"),
                        record.line().unwrap_or(0)
                    )
                })
                .init()
        });

        let config = config.unwrap_or_else(|| NodeConfig::default());

        Self { config }
    }

    pub fn first(mut self) -> Self {
        self.config.first = true;
        self
    }

    pub fn with_contact(mut self, contact: SocketAddr) -> Self {
        let mut contacts = HashSet::default();
        contacts.insert(contact);
        self.config.transport_config.hard_coded_contacts = contacts;
        self
    }

    pub fn elder_size(&mut self, size: usize) -> &mut Self {
        self.config.network_params.elder_size = size;
        self
    }

    pub fn recommended_section_size(&mut self, size: usize) -> &mut Self {
        self.config.network_params.recommended_section_size = size;
        self
    }

    pub fn transport_config(mut self, config: TransportConfig) -> Self {
        self.config.transport_config = config;
        self
    }

    pub fn full_id(mut self, full_id: FullId) -> Self {
        self.config.full_id = Some(full_id);
        self
    }

    pub async fn create(self) -> Result<(Node, EventStream)> {
        // make sure we set 127.0.0.1 as the IP if was not set
        let config = if self.config.transport_config.ip.is_none() {
            let mut config = self.config;
            config.transport_config.ip = Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
            config
        } else {
            self.config
        };
        let node = Node::new(config).await?;
        let event_stream = node.listen_events().await?;

        Ok((node, event_stream))
    }
}

/// Expect that the next event raised by the node matches the given pattern.
/// Errors if no event, or an event that does not match the pattern is raised.
#[macro_export]
macro_rules! expect_next_event {
    ($node:expr, $pattern:pat) => {
        match $node.next().await {
            Some($pattern) => Ok(()),
            other => Err(Error::Unexpected(format!(
                "Expecting {}, got {:?}",
                stringify!($pattern),
                other
            ))),
        }
    };
}

pub async fn verify_invariants_for_node(node: &Node, elder_size: usize) -> Result<()> {
    let our_name = node.name().await;
    assert!(node.matches_our_prefix(&our_name).await?);

    let our_prefix = node
        .our_prefix()
        .await
        .ok_or(Error::Unexpected("Failed to get node's prefix".to_string()))?;

    let our_section_elders: BTreeSet<_> = node
        .our_section()
        .await
        .ok_or(Error::Unexpected("Failed to get node's prefix".to_string()))?
        .elders
        .keys()
        .copied()
        .collect();

    if !our_prefix.is_empty() {
        assert!(
            our_section_elders.len() >= elder_size,
            "{}({:b}) Our section is below the minimum size ({}/{})",
            our_name,
            our_prefix,
            our_section_elders.len(),
            elder_size,
        );
    }

    if let Some(name) = our_section_elders
        .iter()
        .find(|name| !our_prefix.matches(name))
    {
        panic!(
            "{}({:b}) A name in our section doesn't match its prefix: {}",
            our_name, our_prefix, name,
        );
    }

    if !node.is_elder().await {
        return Ok(());
    }

    Ok(())
    /*
    let neighbour_sections: BTreeSet<_> = node.inner.neighbour_sections().collect();

    if let Some(compatible_prefix) = neighbour_sections
        .iter()
        .map(|info| &info.prefix)
        .find(|prefix| prefix.is_compatible(our_prefix))
    {
        panic!(
            "{}({:b}) Our prefix is compatible with one of the neighbour prefixes: {:?} (neighbour_sections: {:?})",
            our_name,
            our_prefix,
            compatible_prefix,
            neighbour_sections,
        );
    }

    if let Some(info) = neighbour_sections
        .iter()
        .find(|info| info.elders.len() < env.elder_size())
    {
        panic!(
            "{}({:b}) A neighbour section {:?} is below the minimum size ({}/{}) (neighbour_sections: {:?})",
            our_name,
            our_prefix,
            info.prefix,
            info.elders.len(),
            env.elder_size(),
            neighbour_sections,
        );
    }

    for info in &neighbour_sections {
        if let Some(name) = info.elders.keys().find(|name| !info.prefix.matches(name)) {
            panic!(
                "{}({:b}) A name in a section doesn't match its prefix: {:?}, {:?}",
                our_name, our_prefix, name, info.prefix,
            );
        }
    }

    let non_neighbours: Vec<_> = neighbour_sections
        .iter()
        .map(|info| &info.prefix)
        .filter(|prefix| !our_prefix.is_neighbour(prefix))
        .collect();
    if !non_neighbours.is_empty() {
        panic!(
            "{}({:b}) Some of our known sections aren't neighbours of our section: {:?}",
            our_name, our_prefix, non_neighbours,
        );
    }

    let all_neighbours_covered = {
        (0..our_prefix.bit_count()).all(|i| {
            our_prefix
                .with_flipped_bit(i as u8)
                .is_covered_by(neighbour_sections.iter().map(|info| &info.prefix))
        })
    };
    if !all_neighbours_covered {
        panic!(
            "{}({:b}) Some neighbours aren't fully covered by our known sections: {:?}",
            our_name,
            our_prefix,
            iter::once(*our_prefix)
                .chain(neighbour_sections.iter().map(|info| info.prefix))
                .format(", ")
        );
    }
    */
}
