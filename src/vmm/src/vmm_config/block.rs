use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use devices::virtio::{block::ImageType, Block, CacheType};

#[derive(Debug)]
pub enum BlockConfigError {
    /// Failed to create the block device.
    CreateBlockDevice(std::io::Error),
}

impl fmt::Display for BlockConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::BlockConfigError::*;
        match *self {
            CreateBlockDevice(ref e) => write!(f, "Cannot create block device: {:?}", e),
        }
    }
}

type Result<T> = std::result::Result<T, BlockConfigError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockDeviceConfig {
    pub block_id: String,
    pub cache_type: CacheType,
    pub device: DeviceKind,
    pub is_disk_read_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    File {
        disk_image_path: String,
        disk_image_format: ImageType,
    },
    #[cfg(unix)]
    CustomIO(devices::CustomIOOptions),
}

#[derive(Default)]
pub struct BlockBuilder {
    pub list: VecDeque<Arc<Mutex<Block>>>,
}

impl BlockBuilder {
    pub fn new() -> Self {
        Self {
            list: VecDeque::<Arc<Mutex<Block>>>::new(),
        }
    }

    pub fn insert(&mut self, config: BlockDeviceConfig) -> Result<()> {
        let block_dev = Arc::new(Mutex::new(Self::create_block(config)?));
        self.list.push_back(block_dev);
        Ok(())
    }

    pub fn create_block(config: BlockDeviceConfig) -> Result<Block> {
        match config.device {
            DeviceKind::File {
                disk_image_path,
                disk_image_format,
            } => devices::virtio::Block::from_file(
                config.block_id,
                None,
                config.cache_type,
                disk_image_path,
                disk_image_format,
                config.is_disk_read_only,
            ),
            #[cfg(unix)]
            DeviceKind::CustomIO(opts) => devices::virtio::Block::from_custom_io(
                config.block_id,
                None,
                config.cache_type,
                opts,
                config.is_disk_read_only,
            ),
        }
        .map_err(BlockConfigError::CreateBlockDevice)
    }
}
