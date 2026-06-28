//! Owned `GrB_Vector` handle with RAII drop semantics.

use crate::context::ensure_init;
use crate::error::{check, Result};
use crate::sys;
use crate::types::ElementType;
use std::ptr;

/// An owned GraphBLAS vector. Freed with `GrB_Vector_free` on drop.
pub struct Vector {
    pub(crate) raw: sys::GrB_Vector,
}

impl Vector {
    /// Create an empty length-`n` vector of the given element type.
    pub fn new(ty: ElementType, n: u64) -> Result<Self> {
        ensure_init();
        let mut raw: sys::GrB_Vector = ptr::null_mut();
        check("GrB_Vector_new", unsafe {
            sys::GrB_Vector_new(&mut raw, ty.grb_type(), n)
        })?;
        Ok(Self { raw })
    }

    /// Create an empty boolean vector.
    pub fn new_bool(n: u64) -> Result<Self> {
        Self::new(ElementType::Bool, n)
    }

    /// Set `v(i) = x` (boolean).
    pub fn set_bool(&mut self, i: u64, x: bool) -> Result<()> {
        check("GrB_Vector_setElement_BOOL", unsafe {
            sys::GrB_Vector_setElement_BOOL(self.raw, x, i)
        })
    }

    /// Read `v(i)`; `Ok(None)` if the entry is not stored (`GrB_NO_VALUE`).
    pub fn get_bool(&self, i: u64) -> Result<Option<bool>> {
        let mut x = false;
        let info = unsafe { sys::GrB_Vector_extractElement_BOOL(&mut x, self.raw, i) };
        if info == sys::GrB_Info_GrB_NO_VALUE {
            return Ok(None);
        }
        check("GrB_Vector_extractElement_BOOL", info)?;
        Ok(Some(x))
    }

    /// Set `v(i) = x` (f64).
    pub fn set_fp64(&mut self, i: u64, x: f64) -> Result<()> {
        check("GrB_Vector_setElement_FP64", unsafe {
            sys::GrB_Vector_setElement_FP64(self.raw, x, i)
        })
    }

    /// Read `v(i)` (f64); `Ok(None)` if the entry is not stored.
    pub fn get_fp64(&self, i: u64) -> Result<Option<f64>> {
        let mut x = 0.0f64;
        let info = unsafe { sys::GrB_Vector_extractElement_FP64(&mut x, self.raw, i) };
        if info == sys::GrB_Info_GrB_NO_VALUE {
            return Ok(None);
        }
        check("GrB_Vector_extractElement_FP64", info)?;
        Ok(Some(x))
    }

    /// Read `v(i)` (i64); `Ok(None)` if the entry is not stored.
    pub fn get_int64(&self, i: u64) -> Result<Option<i64>> {
        let mut x = 0i64;
        let info = unsafe { sys::GrB_Vector_extractElement_INT64(&mut x, self.raw, i) };
        if info == sys::GrB_Info_GrB_NO_VALUE {
            return Ok(None);
        }
        check("GrB_Vector_extractElement_INT64", info)?;
        Ok(Some(x))
    }

    /// Number of stored (explicit) entries.
    pub fn nvals(&self) -> Result<u64> {
        let mut n: sys::GrB_Index = 0;
        check("GrB_Vector_nvals", unsafe {
            sys::GrB_Vector_nvals(&mut n, self.raw)
        })?;
        Ok(n)
    }

    /// The stored indices of a boolean vector (the "set" entries), ascending.
    pub fn indices_bool(&self) -> Result<Vec<u64>> {
        let n = self.nvals()? as usize;
        let mut idx = vec![0u64; n];
        let mut vals = vec![false; n];
        let mut nout: sys::GrB_Index = n as u64;
        check("GrB_Vector_extractTuples_BOOL", unsafe {
            sys::GrB_Vector_extractTuples_BOOL(idx.as_mut_ptr(), vals.as_mut_ptr(), &mut nout, self.raw)
        })?;
        idx.truncate(nout as usize);
        Ok(idx)
    }

    /// Wrap an externally-created `GrB_Vector` (e.g. a LAGraph output vector).
    /// Takes ownership: freed on drop.
    pub(crate) fn from_raw(raw: sys::GrB_Vector) -> Self {
        Self { raw }
    }

    /// Dense read of a length-`n` i64 vector, filling absent entries with
    /// `missing` (LAGraph BFS levels/parents and components are sparse for
    /// unreached nodes).
    pub fn to_dense_i64(&self, n: u64, missing: i64) -> Result<Vec<i64>> {
        let mut out = vec![missing; n as usize];
        for i in 0..n {
            if let Some(v) = self.get_int64(i)? {
                out[i as usize] = v;
            }
        }
        Ok(out)
    }

    /// Dense read of a length-`n` f64 vector, filling absent entries with
    /// `missing`.
    pub fn to_dense_f64(&self, n: u64, missing: f64) -> Result<Vec<f64>> {
        let mut out = vec![missing; n as usize];
        for i in 0..n {
            if let Some(v) = self.get_fp64(i)? {
                out[i as usize] = v;
            }
        }
        Ok(out)
    }

    pub(crate) fn as_raw(&self) -> sys::GrB_Vector {
        self.raw
    }
}

impl Drop for Vector {
    fn drop(&mut self) {
        unsafe {
            sys::GrB_Vector_free(&mut self.raw);
        }
    }
}

// SAFETY: see the note on `Matrix`. A GrB_Vector handle has no thread affinity
// and may move between threads under external synchronization; not Sync.
unsafe impl Send for Vector {}
