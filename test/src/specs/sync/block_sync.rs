use crate::utils::{build_block, build_header, new_block_with_template, wait_until};
use crate::{Net, Node, Spec, TestProtocol};
use ckb_core::block::Block;
use ckb_network::{PeerId, PeerIndex};
use ckb_protocol::{get_root, SyncMessage, SyncPayload};
use ckb_sync::NetworkProtocol;
use jsonrpc_types::{BlockNumber, ChainInfo, Timestamp};
use log::info;
use std::collections::HashSet;
use std::thread::sleep;
use std::time::Duration;

pub struct BlockSyncBasic;

impl BlockSyncBasic {
    // NOTE: ENSURE node0 and nodes1 is in genesis state.
    fn test_sync_from_one(&self, _net: &Net, node0: &Node, node1: &Node) {
        let (mut rpc_client0, mut rpc_client1) = (node0.rpc_client(), node1.rpc_client());
        assert_eq!(
            0,
            rpc_client0
                .get_tip_block_number()
                .call()
                .expect("rpc call get_tip_block_number")
                .0
        );
        assert_eq!(
            0,
            rpc_client1
                .get_tip_block_number()
                .call()
                .expect("rpc call get_tip_block_number")
                .0
        );

        (0..3).for_each(|_| {
            node0.generate_block();
        });

        info!("Connect node0 to node1");
        node0.connect(node1);

        info!("Node1 should be synced to same block number (3) with node0");
        let ret = wait_until(60, || {
            let header0 = rpc_client0
                .get_tip_header()
                .call()
                .expect("rpc call get_tip_header");
            let header1 = rpc_client1
                .get_tip_header()
                .call()
                .expect("rpc call get_tip_header");
            header0 == header1 && header0.inner.number.0 == 3
        });
        assert!(
            ret,
            "Node0 and node1 should sync with each other until same tip chain",
        );
    }

    // NOTE: ENSURE node0 and nodes1 is in genesis state.
    fn test_sync_forks(&self, _net: &Net, node0: &Node, node1: &Node) {
        let (mut rpc_client0, mut rpc_client1) = (node0.rpc_client(), node1.rpc_client());
        assert_eq!(
            0,
            rpc_client0
                .get_tip_block_number()
                .call()
                .expect("rpc call get_tip_block_number")
                .0
        );
        assert_eq!(
            0,
            rpc_client1
                .get_tip_block_number()
                .call()
                .expect("rpc call get_tip_block_number")
                .0
        );

        build_forks(node0, &[2, 0, 0, 0, 0, 0, 0, 0, 0]);
        build_forks(node1, &[1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let info0: ChainInfo = rpc_client0
            .get_blockchain_info()
            .call()
            .expect("rpc call get_blockchain_info");
        let info1: ChainInfo = rpc_client1
            .get_blockchain_info()
            .call()
            .expect("rpc call get_blockchain_info");
        let tip0 = rpc_client0
            .get_tip_header()
            .call()
            .expect("rpc call get_tip_block_number");
        let tip1 = rpc_client1
            .get_tip_header()
            .call()
            .expect("rpc call get_tip_block_number");
        assert_eq!(tip0.inner.number, tip1.inner.number);
        assert_ne!(tip0.hash, tip1.hash);

        // Connect node0 and node1, so that they can sync with each other
        node0.connect(node1);
        let ret = wait_until(60, || {
            let header0 = rpc_client0
                .get_tip_header()
                .call()
                .expect("rpc call get_tip_header");
            let header1 = rpc_client1
                .get_tip_header()
                .call()
                .expect("rpc call get_tip_header");
            header0 == header1
        });
        assert!(
            ret,
            "Node0 and node1 should sync with each other until same tip chain",
        );
        for number in 1u64..tip0.inner.number.0 {
            let block0 = rpc_client0
                .get_block_by_number(BlockNumber(number))
                .call()
                .expect("rpc call get_block_by_number");
            let block1 = rpc_client1
                .get_block_by_number(BlockNumber(number))
                .call()
                .expect("rpc call get_block_by_number");
            assert_eq!(
                block0, block1,
                "nodes should have same best chain after synchronizing",
            );
        }
        let info00: ChainInfo = rpc_client0
            .get_blockchain_info()
            .call()
            .expect("rpc call get_blockchain_info");
        let info11: ChainInfo = rpc_client1
            .get_blockchain_info()
            .call()
            .expect("rpc call get_blockchain_info");
        let medians = vec![
            info0.median_time.0,
            info00.median_time.0,
            info1.median_time.0,
            info11.median_time.0,
        ]
        .into_iter()
        .collect::<HashSet<u64>>();
        assert_eq!(info00.median_time, info11.median_time);
        assert_eq!(medians.len(), 2);
    }

    // Case: Sync a header, sync a duplicated header, reconnect and sync a duplicated header
    pub fn test_sync_duplicated_and_reconnect(&self, net: &Net, node: &Node) {
        net.connect(node);
        let (peer_id, _, _) = net
            .receive_timeout(Duration::new(10, 0))
            .expect("build connection with node");

        // Exit IBD mode
        node.generate_block();

        // Sync a new header to `node`, `node` should send back a corresponding GetBlocks message
        let block = node.new_block(None, None, None);
        sync_header(net, peer_id, &block);
        let (_, _, data) = net
            .receive_timeout(Duration::new(10, 0))
            .expect("Expect SyncMessage");
        let message = get_root::<SyncMessage>(&data).unwrap();
        assert_eq!(
            message.payload_type(),
            SyncPayload::GetBlocks,
            "Node should send back GetBlocks message for the block {:x}",
            block.header().hash(),
        );

        // Sync duplicated header again, `node` should discard the duplicated one.
        // So we will not receive any response messages
        sync_header(net, peer_id, &block);
        assert!(
            net.receive_timeout(Duration::new(10, 0)).is_err(),
            "node should discard duplicated sync headers",
        );

        // Disconnect and reconnect node, and then sync the same header
        // `node` should send back a corresponding GetBlocks message
        let mut rpc_client = node.rpc_client();
        let node_info: jsonrpc_types::Node = rpc_client
            .local_node_info()
            .call()
            .expect("rpc call local_node_info");
        if let Some(ref ctrl) = net.controller.as_ref() {
            ctrl.0
                .remove_node(&PeerId::from_bytes(node_info.node_id.into_bytes()).unwrap());
        }
        net.connect(node);
        let (peer_id, _, _) = net
            .receive_timeout(Duration::new(10, 0))
            .expect("build connection with node");
        sync_header(net, peer_id, &block);
        let (_, _, data) = net
            .receive_timeout(Duration::new(10, 0))
            .expect("Expect SyncMessage");
        let message = get_root::<SyncMessage>(&data).unwrap();
        assert_eq!(
            message.payload_type(),
            SyncPayload::GetBlocks,
            "Node should send back GetBlocks message for the block {:x}",
            block.header().hash(),
        );

        // Sync corresponding block entity, `node` should accept the block as tip block
        sync_block(net, peer_id, &block);
        let hash = block.header().hash().clone();
        wait_until(10, || {
            rpc_client
                .get_tip_header()
                .call()
                .expect("rpc call get_tip_header")
                .hash
                == hash
        });
    }

    pub fn test_sync_orphan_blocks(&self, net: &Net, node0: &Node, node1: &Node) {
        // Exit IBD mode
        node0.generate_block();
        net.connect(node0);
        let (peer_id, _, _) = net
            .receive_timeout(Duration::new(10, 0))
            .expect("net receive timeout");
        let mut rpc_client = node0.rpc_client();
        let tip_number = rpc_client.get_tip_block_number().call().unwrap();

        // Generate some blocks from node1
        let mut blocks: Vec<Block> = (1..=5)
            .into_iter()
            .map(|_| {
                let block = node1.new_block(None, None, None);
                node1.submit_block(&block);
                block
            })
            .collect();

        // Send headers to node0, keep blocks body
        blocks.iter().for_each(|block| {
            sync_header(net, peer_id, block);
        });

        // Wait for block fetch timer
        sleep(Duration::from_secs(5));

        // Skip the next block, send the rest blocks to node0
        let first = blocks.remove(0);
        blocks.into_iter().for_each(|block| {
            sync_block(net, peer_id, &block);
        });
        let ret = wait_until(5, || {
            rpc_client.get_tip_block_number().call().unwrap().0 > tip_number.0
        });
        assert!(!ret, "node0 should stay the same");

        // Send that skipped first block to node0
        sync_block(net, peer_id, &first);
        let ret = wait_until(10, || {
            rpc_client.get_tip_block_number().call().unwrap().0 > tip_number.0 + 2
        });
        assert!(ret, "node0 should grow up");
    }
}

impl Spec for BlockSyncBasic {
    fn run(&self, net: Net) {
        info!("Running BlockSyncBasic");
        self.test_sync_forks(&net, &net.nodes[0], &net.nodes[1]);
        self.test_sync_from_one(&net, &net.nodes[2], &net.nodes[3]);
        self.test_sync_duplicated_and_reconnect(&net, &net.nodes[4]);
        self.test_sync_orphan_blocks(&net, &net.nodes[5], &net.nodes[6]);
    }

    fn num_nodes(&self) -> usize {
        7
    }

    fn connect_all(&self) -> bool {
        false
    }

    fn test_protocols(&self) -> Vec<TestProtocol> {
        vec![TestProtocol::sync()]
    }
}

fn build_forks(node: &Node, offsets: &[u64]) {
    let mut rpc_client = node.rpc_client();
    for offset in offsets.iter() {
        let mut template = rpc_client
            .get_block_template(None, None, None)
            .call()
            .expect("rpc call get_block_template failed");
        template.current_time = Timestamp(template.current_time.0 + offset);
        let block = new_block_with_template(template);
        node.submit_block(&block);
    }
}

fn sync_header(net: &Net, peer_id: PeerIndex, block: &Block) {
    net.send(
        NetworkProtocol::SYNC.into(),
        peer_id,
        build_header(block.header()),
    );
}

fn sync_block(net: &Net, peer_id: PeerIndex, block: &Block) {
    net.send(NetworkProtocol::SYNC.into(), peer_id, build_block(block));
}
