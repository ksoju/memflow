/*!
Basic connector which works on mapped memory.
*/

use crate::iter::FnExtend;
use crate::mem::cloneable_slice::CloneableSliceMut;
use crate::mem::phys_mem::{PoolablePhysicalMemory, PooledPhysicalMemory};
use crate::mem::{MemoryMap, PhysicalMemory, PhysicalReadData, PhysicalWriteData};
use crate::types::{Address, TryAsMut};
use crate::{Error, Result};

#[cfg(feature = "filemap")]
use {
    memmap::{Mmap, MmapMut, MmapOptions},
    std::fs::File,
    std::sync::Arc,
};

pub struct MMAPInfo<'a> {
    mem_map: MemoryMap<&'a [u8]>,
    #[cfg(feature = "filemap")]
    _buf: Arc<Mmap>,
}

#[cfg(feature = "filemap")]
impl<'a> AsRef<MemoryMap<&'a [u8]>> for MMAPInfo<'a> {
    fn as_ref(&self) -> &MemoryMap<&'a [u8]> {
        &self.mem_map
    }
}

pub struct MMAPInfoMut<'a> {
    mem_map: MemoryMap<CloneableSliceMut<'a, u8>>,
    #[cfg(feature = "filemap")]
    _buf: MmapMut,
}

#[cfg(feature = "filemap")]
impl<'a> AsRef<MemoryMap<CloneableSliceMut<'a, u8>>> for MMAPInfoMut<'a> {
    fn as_ref(&self) -> &MemoryMap<CloneableSliceMut<'a, u8>> {
        &self.mem_map
    }
}

pub struct MappedPhysicalMemory<T, F> {
    info: F,
    marker: std::marker::PhantomData<T>,
}

impl<T, F: Clone> Clone for MappedPhysicalMemory<T, F> {
    fn clone(&self) -> Self {
        Self {
            info: self.info.clone(),
            marker: Default::default(),
        }
    }
}

impl<T: Send + 'static, F: Clone + Send + 'static> PoolablePhysicalMemory
    for MappedPhysicalMemory<T, F>
where
    MappedPhysicalMemory<T, F>: PhysicalMemory,
{
    fn make_phys_pool<'a>(&'a self, size_hint: usize) -> Vec<PooledPhysicalMemory<'a>> {
        (0..size_hint)
            .map(|_| Box::new(self.clone()) as PooledPhysicalMemory<'a>)
            .collect()
    }
}

impl MappedPhysicalMemory<&'static mut [u8], MemoryMap<&'static mut [u8]>> {
    /// Create a connector using virtual address mappings
    ///
    /// # Safety
    ///
    /// This connector assumes the memory map is valid, and writeable. Failure for these conditions
    /// to be met leads to undefined behaviour (most likely a segfault) when reading/writing.
    pub unsafe fn from_addrmap_mut(map: MemoryMap<(Address, usize)>) -> Self {
        Self::with_info(map.into_bufmap_mut::<'static>())
    }
}

impl MappedPhysicalMemory<&'static [u8], MemoryMap<&'static [u8]>> {
    /// Create a connector using virtual address mappings
    ///
    /// # Safety
    ///
    /// This connector assumes the memory map is valid. Failure for this condition to be met leads
    /// to undefined behaviour (most likely a segfault) when reading.
    pub unsafe fn from_addrmap(map: MemoryMap<(Address, usize)>) -> Self {
        Self::with_info(map.into_bufmap::<'static>())
    }
}

impl<T: AsRef<[u8]>, F: AsRef<MemoryMap<T>>> MappedPhysicalMemory<T, F> {
    pub fn with_info(info: F) -> Self {
        Self {
            info,
            marker: Default::default(),
        }
    }
}

#[cfg(feature = "filemap")]
impl<'a> MappedPhysicalMemory<&'a [u8], MMAPInfo<'a>> {
    pub fn try_with_filemap(file: File, map: MemoryMap<(Address, usize)>) -> Result<Self> {
        let file_map = unsafe {
            MmapOptions::new()
                .map(&file)
                .map_err(|_| Error::Connector("unable to map file"))?
        };

        Self::try_with_bufmap(file_map, map)
    }
}

#[cfg(feature = "filemap")]
impl<'a> MappedPhysicalMemory<CloneableSliceMut<'a, u8>, MMAPInfoMut<'a>> {
    pub fn try_with_filemap_mut(file: File, map: MemoryMap<(Address, usize)>) -> Result<Self> {
        let file_map = unsafe {
            MmapOptions::new()
                .map_mut(&file)
                .map_err(|_| Error::Connector("unable to map file"))?
        };

        Self::try_with_bufmap_mut(file_map, map)
    }
}

pub type ReadMappedFilePhysicalMemory<'a> = MappedPhysicalMemory<&'a [u8], MMAPInfo<'a>>;

#[cfg(feature = "filemap")]
impl<'a> ReadMappedFilePhysicalMemory<'a> {
    pub fn try_with_bufmap(buf: Mmap, map: MemoryMap<(Address, usize)>) -> Result<Self> {
        let mut new_map = MemoryMap::new();

        let buf_len = buf.as_ref().len();
        let buf_ptr = buf.as_ref().as_ptr();

        for (base, (output_base, size)) in map.into_iter() {
            if output_base.as_usize() >= buf_len {
                return Err(Error::Connector("Memory map is out of range"));
            }

            let output_end = std::cmp::min(output_base.as_usize() + size, buf_len);

            new_map.push(base, unsafe {
                std::slice::from_raw_parts(
                    buf_ptr.add(output_base.as_usize()),
                    output_end - output_base.as_usize(),
                )
            });
        }

        Ok(Self::with_info(MMAPInfo {
            mem_map: new_map,
            _buf: Arc::new(buf),
        }))
    }
}

pub type WriteMappedFilePhysicalMemory<'a> =
    MappedPhysicalMemory<CloneableSliceMut<'a, u8>, MMAPInfoMut<'a>>;

//TODO: Dedup this code. And make it safer?
#[cfg(feature = "filemap")]
impl<'a> WriteMappedFilePhysicalMemory<'a> {
    pub fn try_with_bufmap_mut(mut buf: MmapMut, map: MemoryMap<(Address, usize)>) -> Result<Self> {
        let mut new_map = MemoryMap::new();

        let buf_len = buf.as_ref().len();
        let buf_ptr = buf.as_mut().as_mut_ptr();

        for (base, (output_base, size)) in map.into_iter() {
            if output_base.as_usize() >= buf_len {
                return Err(Error::Connector("Memory map is out of range"));
            }

            let output_end = std::cmp::min(output_base.as_usize() + size, buf_len);

            new_map.push(base, unsafe {
                CloneableSliceMut::from_slice_mut(std::slice::from_raw_parts_mut(
                    buf_ptr.add(output_base.as_usize()),
                    output_end - output_base.as_usize(),
                ))
            });
        }

        Ok(Self::with_info(MMAPInfoMut {
            mem_map: new_map,
            _buf: buf,
        }))
    }
}

impl<
        'a,
        T: TryAsMut<[u8]> + AsRef<[u8]> + crate::iter::SplitAtIndexNoMutation + Send,
        F: AsRef<MemoryMap<T>> + Send,
    > PhysicalMemory for MappedPhysicalMemory<T, F>
{
    fn phys_read_raw_list(&mut self, data: &mut [PhysicalReadData]) -> Result<()> {
        let mut void = FnExtend::void();
        for (mapped_buf, buf) in self.info.as_ref().map_iter(
            data.iter_mut().map(|(addr, buf)| (*addr, &mut **buf)),
            &mut void,
        ) {
            buf.copy_from_slice(mapped_buf.as_ref());
        }
        Ok(())
    }

    fn phys_write_raw_list(&mut self, data: &[PhysicalWriteData]) -> Result<()> {
        let mut void = FnExtend::void();
        for (mut mapped_buf, buf) in self.info.as_ref().map_iter(data.iter().copied(), &mut void) {
            mapped_buf
                .try_as_mut()
                .ok_or(Error::Connector("Target writing is not supported"))?
                .copy_from_slice(buf);
        }

        Ok(())
    }
}

/*impl<'a, T: OnlyAsRef<[u8]>, F: AsRef<MemoryMap<&'a [u8]>>> PhysicalMemory for MappedPhysicalMemory<T, F> {
    fn phys_read_raw_list(&mut self, data: &mut [PhysicalReadData]) -> Result<()> {
        let mut void = FnExtend::void();
        for (mapped_buf, buf) in self.info.as_ref().map_iter(
            data.iter_mut().map(|(addr, buf)| (*addr, &mut **buf)),
            &mut void,
        ) {
            buf.copy_from_slice(mapped_buf);
        }
        Ok(())
    }

    fn phys_write_raw_list(&mut self, _data: &[PhysicalWriteData]) -> Result<()> {
        Err(Error::Connector("Target mapping is not writeable"))
    }
}*/
