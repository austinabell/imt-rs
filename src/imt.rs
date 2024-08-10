use std::collections::HashMap;
use tiny_keccak::{Hasher, Keccak};

use crate::{
    circuits::{
        mutate::IMTMutate,
        node::{Hashor, IMTNode, Key, Value},
    },
    Hash,
};

#[derive(Debug)]
pub struct Imt<H: Hashor, K: Key, V: Value> {
    pub root: Hash,

    hasher: H,
    depth: u8,
    size: u64,
    nodes: HashMap<K, IMTNode<K, V>>,
    hashes: HashMap<u8, HashMap<u64, Hash>>,
}

impl<H: Hashor, K: Key, V: Value> Imt<H, K, V> {
    pub fn new(hasher: H, depth: u8) -> Self {
        let mut imt = Self {
            root: Default::default(),

            hasher,
            size: Default::default(),
            depth,
            nodes: Default::default(),
            hashes: Default::default(),
        };

        let init_node_key = K::default();
        let init_node = IMTNode {
            index: Default::default(),
            key: Default::default(),
            value: Default::default(),
            next_key: Default::default(),
        };
        imt.nodes.insert(init_node_key, init_node);
        imt.refresh_tree(&init_node_key);

        imt
    }

    pub fn insert_node(&mut self, key: K, value: V) -> IMTMutate<K, V> {
        // Do not overflow the tree.
        let max_size = (1 << self.depth) - 1;
        assert!(self.size < max_size, "tree overflow");

        // Ensure key does not already exist in the tree.
        assert!(!self.nodes.contains_key(&key), "key conflict");

        let old_root = self.root;
        let old_size = self.size;

        let node_index = {
            self.size += 1;
            self.size
        };

        // Get the ln node.
        let ln_node = self.low_nullifier(&key);
        let ln_siblings = self.siblings(&ln_node.key);

        // Update the ln node and refresh the tree.
        self.nodes
            .get_mut(&ln_node.key)
            .expect("failed to get node")
            .next_key = key;
        self.refresh_tree(&ln_node.key);

        // Create the new node.
        let node = IMTNode {
            index: node_index,
            key,
            value,
            next_key: ln_node.next_key,
        };

        // Insert the new node and refresh the tree.
        self.nodes.insert(node.key, node);
        let node_siblings = self.refresh_tree(&key);

        let updated_ln_siblings = self.siblings(&ln_node.key);

        // Return the IMTMutate insertion to use for proving.
        IMTMutate::insert(
            old_root,
            old_size,
            ln_node,
            ln_siblings,
            node,
            node_siblings,
            updated_ln_siblings,
        )
    }

    pub fn update_node(&mut self, key: K, value: V) -> IMTMutate<K, V> {
        let node = self.nodes.get_mut(&key).expect("node does not exist");

        let old_root = self.root;
        let size = self.size;

        node.value = value;
        let node = *node;

        let node_siblings = self.refresh_tree(&key);

        IMTMutate::update(old_root, size, node, node_siblings, value)
    }

    fn refresh_tree(&mut self, node_key: &K) -> Vec<Option<Hash>> {
        let node = self.nodes.get(node_key).expect("failed to get node");
        let mut index = node.index;

        // Recompute and cache the node hash.
        let mut hash = node.hash(self.hasher.clone());
        self.hashes
            .entry(self.depth)
            .or_default()
            .insert(index, hash);

        // Climb up the tree and refresh the hashes.
        let mut siblings = Vec::with_capacity(self.depth.into());
        for level in (1..=self.depth).rev() {
            let sibling_index = index + (1 - 2 * (index % 2));
            let sibling_hash = self
                .hashes
                .entry(level)
                .or_default()
                .get(&sibling_index)
                .cloned();

            siblings.push(sibling_hash);

            let (left, right) = if index % 2 == 0 {
                (Some(hash), sibling_hash)
            } else {
                (sibling_hash, Some(hash))
            };

            let mut k = Keccak::v256();
            match (left, right) {
                (None, None) => unreachable!(),
                (None, Some(right)) => k.update(&right),
                (Some(left), None) => k.update(&left),
                (Some(left), Some(right)) => {
                    k.update(&left);
                    k.update(&right);
                }
            };

            k.finalize(&mut hash);

            index /= 2;

            self.hashes
                .entry(level - 1)
                .or_default()
                .insert(index, hash);
        }

        // Refresh the root hash.
        self.root = {
            let mut root_hash = [0; 32];

            let mut k = Keccak::v256();
            k.update(&hash);
            k.update(&self.size.to_be_bytes());
            k.finalize(&mut root_hash);

            root_hash
        };

        siblings
    }

    fn low_nullifier(&self, node_key: &K) -> IMTNode<K, V> {
        let ln = self
            .nodes
            .values()
            .find(|node| node.is_ln_of(node_key))
            .expect("failed to found ln node");

        *ln
    }

    fn siblings(&self, node_key: &K) -> Vec<Option<Hash>> {
        let node = self.nodes.get(node_key).expect("node does not exist");

        let mut siblings = Vec::with_capacity(self.depth.into());
        let mut index = node.index;

        for level in (1..=self.depth).rev() {
            let sibling_index = index + (1 - 2 * (index % 2));
            let sibling_hash = self
                .hashes
                .get(&level)
                .and_then(|m| m.get(&sibling_index).cloned());

            siblings.push(sibling_hash);
            index /= 2;
        }

        siblings
    }
}
