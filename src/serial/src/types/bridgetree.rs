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

use core::fmt::Debug;
use std::io::{Error, ErrorKind, Read, Write};

use crate::{Decodable, Encodable};

impl Encodable for bridgetree::Position {
    fn encode<S: Write>(&self, mut s: S) -> Result<usize, Error> {
        u64::from(*self).encode(&mut s)
    }
}

impl Decodable for bridgetree::Position {
    fn decode<D: Read>(mut d: D) -> Result<Self, Error> {
        let dec: u64 = Decodable::decode(&mut d)?;
        Ok(Self::try_from(dec).unwrap())
    }
}

impl Encodable for bridgetree::Address {
    fn encode<S: Write>(&self, mut s: S) -> Result<usize, Error> {
        let mut len = 0;
        len += u8::from(self.level()).encode(&mut s)?;
        len += self.index().encode(&mut s)?;
        Ok(len)
    }
}

impl Decodable for bridgetree::Address {
    fn decode<D: Read>(mut d: D) -> Result<Self, Error> {
        let level: u8 = Decodable::decode(&mut d)?;
        let index = Decodable::decode(&mut d)?;
        Ok(Self::from_parts(level.into(), index))
    }
}

impl<H: Encodable + Ord + Clone> Encodable for bridgetree::NonEmptyFrontier<H> {
    fn encode<S: Write>(&self, mut s: S) -> Result<usize, Error> {
        let mut len = 0;
        len += self.position().encode(&mut s)?;
        len += self.leaf().encode(&mut s)?;
        len += self.ommers().to_vec().encode(&mut s)?;
        Ok(len)
    }
}

impl<H: Decodable + Ord + Clone> Decodable for bridgetree::NonEmptyFrontier<H> {
    fn decode<D: Read>(mut d: D) -> Result<Self, Error> {
        let position = Decodable::decode(&mut d)?;
        let leaf = Decodable::decode(&mut d)?;
        let ommers = Decodable::decode(&mut d)?;

        match Self::from_parts(position, leaf, ommers) {
            Ok(v) => Ok(v),
            Err(_) => Err(Error::new(ErrorKind::Other, "FrontierError")),
        }
    }
}

impl<H: Encodable + Ord + Clone> Encodable for bridgetree::MerkleBridge<H> {
    fn encode<S: Write>(&self, mut s: S) -> Result<usize, Error> {
        let mut len = 0;
        len += self.prior_position().encode(&mut s)?;
        len += self.tracking().encode(&mut s)?;
        len += self.ommers().encode(&mut s)?;
        len += self.frontier().encode(&mut s)?;
        Ok(len)
    }
}

impl<H: Decodable + Ord + Clone> Decodable for bridgetree::MerkleBridge<H> {
    fn decode<D: Read>(mut d: D) -> Result<Self, Error> {
        let prior_position = Decodable::decode(&mut d)?;
        let tracking = Decodable::decode(&mut d)?;
        let ommers = Decodable::decode(&mut d)?;
        let frontier = Decodable::decode(&mut d)?;
        Ok(Self::from_parts(prior_position, tracking, ommers, frontier))
    }
}

impl<C: Encodable> Encodable for bridgetree::Checkpoint<C> {
    fn encode<S: Write>(&self, mut s: S) -> Result<usize, Error> {
        let mut len = 0;
        len += self.id().encode(&mut s)?;
        len += self.bridges_len().encode(&mut s)?;
        len += self.marked().encode(&mut s)?;
        len += self.forgotten().encode(&mut s)?;
        Ok(len)
    }
}

impl<C: Decodable> Decodable for bridgetree::Checkpoint<C> {
    fn decode<D: Read>(mut d: D) -> Result<Self, Error> {
        let id = Decodable::decode(&mut d)?;
        let bridges_len = Decodable::decode(&mut d)?;
        let marked = Decodable::decode(&mut d)?;
        let forgotten = Decodable::decode(&mut d)?;
        Ok(Self::from_parts(id, bridges_len, marked, forgotten))
    }
}

impl<H: Encodable + Ord + Clone, C: Encodable + Debug, const DEPTH: u8> Encodable
    for bridgetree::BridgeTree<H, C, DEPTH>
{
    fn encode<S: Write>(&self, mut s: S) -> Result<usize, Error> {
        let mut len = 0;
        len += self.prior_bridges().to_vec().encode(&mut s)?;
        len += self.current_bridge().encode(&mut s)?;
        len += self.marked_indices().encode(&mut s)?;
        len += self.checkpoints().encode(&mut s)?;
        len += self.max_checkpoints().encode(&mut s)?;
        Ok(len)
    }
}

impl<
        H: Decodable + Clone + Ord + bridgetree::Hashable,
        C: Decodable + Clone + Ord + Eq + Debug,
        const DEPTH: u8,
    > Decodable for bridgetree::BridgeTree<H, C, DEPTH>
{
    fn decode<D: Read>(mut d: D) -> Result<Self, Error> {
        let prior_bridges = Decodable::decode(&mut d)?;
        let current_bridge = Decodable::decode(&mut d)?;
        let saved = Decodable::decode(&mut d)?;
        let checkpoints = Decodable::decode(&mut d)?;
        let max_checkpoints = Decodable::decode(&mut d)?;
        match Self::from_parts(prior_bridges, current_bridge, saved, checkpoints, max_checkpoints) {
            Ok(v) => Ok(v),
            Err(_) => Err(Error::new(ErrorKind::Other, "BridgeTreeError")),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{deserialize, serialize, SerialDecodable, SerialEncodable};
    use bridgetree::{BridgeTree, Hashable, Level};

    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, SerialEncodable, SerialDecodable)]
    struct Node(String);

    impl Hashable for Node {
        fn empty_leaf() -> Self {
            Self("_".to_string())
        }

        fn combine(_: Level, a: &Self, b: &Self) -> Self {
            Self(a.0.to_string() + &b.0)
        }
    }

    #[test]
    fn serialize_desrialize_inc_merkle_tree() {
        const DEPTH: u8 = 8;

        // Fill the tree with 100 leaves
        let mut tree: BridgeTree<Node, usize, DEPTH> = BridgeTree::new(100);
        for i in 0..100 {
            tree.append(Node(format!("test{}", i)));
            tree.mark();
            tree.checkpoint(i);
        }
        let serial_tree = serialize(&tree);
        let deserial_tree: BridgeTree<Node, usize, DEPTH> = deserialize(&serial_tree).unwrap();

        // Empty tree
        let tree2: BridgeTree<Node, usize, DEPTH> = BridgeTree::new(100);
        let serial_tree2 = serialize(&tree2);
        let deserial_tree2: BridgeTree<Node, usize, DEPTH> = deserialize(&serial_tree2).unwrap();

        // Max leaves
        let mut tree3: BridgeTree<Node, usize, DEPTH> = BridgeTree::new(100);
        for i in 0..2_i32.pow(DEPTH as u32) {
            tree3.append(Node(format!("test{}", i)));
            tree3.mark();
            tree3.checkpoint(i.try_into().unwrap());
        }
        let serial_tree3 = serialize(&tree3);
        let deserial_tree3: BridgeTree<Node, usize, DEPTH> = deserialize(&serial_tree3).unwrap();

        assert!(tree == deserial_tree);
        assert!(tree2 == deserial_tree2);
        assert!(tree3 == deserial_tree3);
    }
}
