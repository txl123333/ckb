use crate::{
    synchronizer::{BlockStatus, Synchronizer},
    Status, StatusCode,
};
use ckb_logger::debug;
use ckb_network::PeerIndex;
use ckb_types::{packed, prelude::*};
use std::sync::Arc;

pub struct BlockProcess<'a> {
    message: packed::SendBlockReader<'a>,
    synchronizer: &'a Synchronizer,
    peer: PeerIndex,
}

impl<'a> BlockProcess<'a> {
    pub fn new(
        message: packed::SendBlockReader<'a>,
        synchronizer: &'a Synchronizer,
        peer: PeerIndex,
    ) -> Self {
        BlockProcess {
            message,
            synchronizer,
            peer,
        }
    }

    pub fn execute(self) -> Status {
        let block = Arc::new(self.message.block().to_entity().into_view());
        debug!(
            "BlockProcess received block {} {}",
            block.number(),
            block.hash(),
        );
        let shared = self.synchronizer.shared();
        let state = shared.state();

        if state.new_block_received(&block) {
            self.synchronizer.push_block(self.peer, block.clone());
        // if let Err(err) = self
        //     .synchronizer
        //     .process_new_block(self.peer, block.clone())
        // {
        //     state.insert_block_status(block.hash(), BlockStatus::BLOCK_INVALID);
        //     return StatusCode::BlockIsInvalid.with_context(format!(
        //         "{}, error: {}",
        //         block.hash(),
        //         err,
        //     ));
        // }
        } else if shared
            .active_chain()
            .contains_block_status(&block.hash(), BlockStatus::BLOCK_STORED)
        {
            state
                .peers()
                .set_last_common_header(self.peer, block.header());
        }

        Status::ok()
    }
}
