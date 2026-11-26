//! FAT32 filesystem driver.
//!
//! Supports basic directory traversal and file reading from a block device.

use crate::KernelError;

/// FAT32 boot sector (BIOS Parameter Block).
#[derive(Debug, Clone)]
pub struct Fat32BootSector {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub fat_count: u8,
    pub total_sectors_16: u16,
    pub media_descriptor: u8,
    pub sectors_per_fat_16: u16,
    pub sectors_per_track: u16,
    pub heads: u16,
    pub hidden_sectors: u32,
    pub total_sectors_32: u32,
    pub sectors_per_fat_32: u32,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
    pub backup_boot_sector: u16,
}

impl Fat32BootSector {
    /// Parse boot sector from the first 512 bytes of the volume.
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 512 {
            return Err(KernelError::InvalidArg("boot sector too short"));
        }
        // Check signature
        if data[510] != 0x55 || data[511] != 0xAA {
            return Err(KernelError::InvalidArg("invalid boot signature"));
        }
        // Check FAT32 signature (offset 0x42 = 0x28 or 0x29)
        if data[0x42] != 0x28 && data[0x42] != 0x29 {
            return Err(KernelError::InvalidArg("not a FAT32 volume"));
        }
        Ok(Self {
            bytes_per_sector: u16::from_le_bytes([data[11], data[12]]),
            sectors_per_cluster: data[13],
            reserved_sectors: u16::from_le_bytes([data[14], data[15]]),
            fat_count: data[16],
            total_sectors_16: u16::from_le_bytes([data[19], data[20]]),
            media_descriptor: data[21],
            sectors_per_fat_16: u16::from_le_bytes([data[22], data[23]]),
            sectors_per_track: u16::from_le_bytes([data[24], data[25]]),
            heads: u16::from_le_bytes([data[26], data[27]]),
            hidden_sectors: u32::from_le_bytes([data[28], data[29], data[30], data[31]]),
            total_sectors_32: u32::from_le_bytes([data[32], data[33], data[34], data[35]]),
            sectors_per_fat_32: u32::from_le_bytes([data[36], data[37], data[38], data[39]]),
            root_cluster: u32::from_le_bytes([data[44], data[45], data[46], data[47]]),
            fs_info_sector: u16::from_le_bytes([data[48], data[49]]),
            backup_boot_sector: u16::from_le_bytes([data[50], data[51]]),
        })
    }

    /// Total number of sectors in the volume.
    pub fn total_sectors(&self) -> u32 {
        if self.total_sectors_16 != 0 {
            self.total_sectors_16 as u32
        } else {
            self.total_sectors_32
        }
    }

    /// Number of bytes per cluster.
    pub fn bytes_per_cluster(&self) -> u32 {
        self.bytes_per_sector as u32 * self.sectors_per_cluster as u32
    }
}

/// FAT32 filesystem mounted on a block device.
pub struct Fat32Fs {
    boot: Fat32BootSector,
    #[allow(dead_code)]
    fat_start_sector: u32,
    data_start_sector: u32,
    #[allow(dead_code)]
    root_cluster: u32,
}

impl Fat32Fs {
    /// Mount FAT32 filesystem from boot sector data.
    pub fn new(boot: Fat32BootSector) -> Self {
        let fat_start_sector = boot.reserved_sectors as u32;
        let fat_size_sectors = boot.sectors_per_fat_32;
        let data_start_sector = fat_start_sector + fat_size_sectors * boot.fat_count as u32;
        let root_cluster = boot.root_cluster;
        Self {
            boot,
            fat_start_sector,
            data_start_sector,
            root_cluster,
        }
    }

    /// Convert cluster number to data sector number.
    pub fn cluster_to_sector(&self, cluster: u32) -> u32 {
        if cluster < 2 {
            return 0;
        }
        self.data_start_sector + (cluster - 2) * self.boot.sectors_per_cluster as u32
    }

    /// Read FAT entry for a cluster.
    pub fn read_fat_entry(&self, cluster: u32, fat_data: &[u8]) -> u32 {
        let fat_offset = cluster * 4;
        if fat_offset + 4 > fat_data.len() as u32 {
            return 0x0FFFFFF8; // EOF marker
        }
        let entry = u32::from_le_bytes([
            fat_data[fat_offset as usize],
            fat_data[fat_offset as usize + 1],
            fat_data[fat_offset as usize + 2],
            fat_data[fat_offset as usize + 3],
        ]);
        entry & 0x0FFFFFFF
    }

    /// Check if a cluster is the end of a chain.
    pub fn is_eof_cluster(&self, cluster: u32) -> bool {
        cluster >= 0x0FFFFFF8
    }
}

/// FAT32 directory entry (8.3 short name format).
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: [u8; 11],
    pub attributes: u8,
    pub first_cluster: u16,
    pub file_size: u32,
}

impl DirEntry {
    /// Parse a 32-byte directory entry.
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 32 {
            return Err(KernelError::InvalidArg("dir entry too short"));
        }
        // Check for deleted entry
        if data[0] == 0xE5 {
            return Err(KernelError::InvalidArg("deleted entry"));
        }
        // Check for end of directory
        if data[0] == 0x00 {
            return Err(KernelError::InvalidArg("end of directory"));
        }
        let mut name = [0u8; 11];
        name.copy_from_slice(&data[0..11]);
        Ok(Self {
            name,
            attributes: data[11],
            first_cluster: u16::from_le_bytes([data[26], data[27]]),
            file_size: u32::from_le_bytes([data[28], data[29], data[30], data[31]]),
        })
    }

    /// Check if this is a directory.
    pub fn is_directory(&self) -> bool {
        self.attributes & 0x10 != 0
    }

    /// Get short name as a string (without trailing spaces).
    pub fn short_name(&self) -> alloc::string::String {
        let mut s = alloc::string::String::new();
        for i in 0..8 {
            if self.name[i] == b' ' {
                break;
            }
            s.push(self.name[i] as char);
        }
        if self.name[8] != b' ' {
            s.push('.');
            for i in 8..11 {
                if self.name[i] == b' ' {
                    break;
                }
                s.push(self.name[i] as char);
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_sector_parse_minimal() {
        let mut data = [0u8; 512];
        data[0x42] = 0x28; // FAT32 signature
        data[510] = 0x55;
        data[511] = 0xAA;
        data[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes per sector
        data[13] = 0x08; // sectors per cluster = 8
        let boot = Fat32BootSector::parse(&data).unwrap();
        assert_eq!(boot.bytes_per_sector, 512);
        assert_eq!(boot.sectors_per_cluster, 8);
    }

    #[test]
    fn dir_entry_parse() {
        let mut data = [0u8; 32];
        data[0..11].copy_from_slice(b"HELLO   TXT");
        data[11] = 0x20; // archive attribute
        data[26] = 0x02;
        data[27] = 0x00;
        data[28..32].copy_from_slice(&1234u32.to_le_bytes());
        let entry = DirEntry::parse(&data).unwrap();
        assert_eq!(entry.short_name(), "HELLO.TXT");
        assert_eq!(entry.file_size, 1234);
        assert!(!entry.is_directory());
    }
}
