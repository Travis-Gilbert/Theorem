use numpy::{IntoPyArray, PyReadonlyArray1};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;

#[pyfunction]
pub fn graph_remap_ids_batch<'py>(
    py: Python<'py>,
    object_ids: PyReadonlyArray1<'py, i64>,
) -> PyResult<Bound<'py, PyDict>> {
    let ids = object_ids.as_slice()?.to_vec();
    let (dense_ids, unique_ids) = py.allow_threads(move || remap_ids_values(ids));
    let out = PyDict::new_bound(py);
    out.set_item("dense_ids", dense_ids.into_pyarray_bound(py))?;
    out.set_item("unique_ids", unique_ids.into_pyarray_bound(py))?;
    Ok(out)
}

#[pyfunction]
pub fn graph_pack_edges_batch<'py>(
    py: Python<'py>,
    source_ids: PyReadonlyArray1<'py, i64>,
    target_ids: PyReadonlyArray1<'py, i64>,
    relation_ids: PyReadonlyArray1<'py, i32>,
    dense_id_by_object_id: HashMap<i64, i64>,
) -> PyResult<Bound<'py, PyDict>> {
    let src_values = source_ids.as_slice()?.to_vec();
    let dst_values = target_ids.as_slice()?.to_vec();
    let rel_values = relation_ids.as_slice()?.to_vec();
    let (src, dst, rel) = py.allow_threads(move || {
        pack_edges_values(
            &src_values,
            &dst_values,
            &rel_values,
            &dense_id_by_object_id,
        )
    });
    let out = PyDict::new_bound(py);
    out.set_item("src", src.into_pyarray_bound(py))?;
    out.set_item("dst", dst.into_pyarray_bound(py))?;
    out.set_item("rel", rel.into_pyarray_bound(py))?;
    Ok(out)
}

fn remap_ids_values(ids: Vec<i64>) -> (Vec<i64>, Vec<i64>) {
    let mut map: HashMap<i64, i64> = HashMap::with_capacity(ids.len());
    let mut unique: Vec<i64> = Vec::new();
    let mut dense: Vec<i64> = Vec::with_capacity(ids.len());
    for id in ids {
        let next = map.len() as i64;
        let entry = map.entry(id).or_insert_with(|| {
            unique.push(id);
            next
        });
        dense.push(*entry);
    }
    (dense, unique)
}

fn pack_edges_values(
    src_values: &[i64],
    dst_values: &[i64],
    rel_values: &[i32],
    dense_id_by_object_id: &HashMap<i64, i64>,
) -> (Vec<i64>, Vec<i64>, Vec<i32>) {
    let len = src_values.len().min(dst_values.len()).min(rel_values.len());
    let mut src_out: Vec<i64> = Vec::with_capacity(len);
    let mut dst_out: Vec<i64> = Vec::with_capacity(len);
    let mut rel_out: Vec<i32> = Vec::with_capacity(len);
    for idx in 0..len {
        let Some(src_dense) = dense_id_by_object_id.get(&src_values[idx]) else {
            continue;
        };
        let Some(dst_dense) = dense_id_by_object_id.get(&dst_values[idx]) else {
            continue;
        };
        src_out.push(*src_dense);
        dst_out.push(*dst_dense);
        rel_out.push(rel_values[idx]);
    }
    (src_out, dst_out, rel_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remap_ids_preserves_first_seen_order() {
        let (dense, unique) = remap_ids_values(vec![42, 7, 42, -1, 7]);
        assert_eq!(dense, vec![0, 1, 0, 2, 1]);
        assert_eq!(unique, vec![42, 7, -1]);
    }

    #[test]
    fn pack_edges_skips_unknown_and_truncates_to_shortest_input() {
        let mut dense = HashMap::new();
        dense.insert(10, 0);
        dense.insert(20, 1);
        dense.insert(30, 2);
        let (src, dst, rel) =
            pack_edges_values(&[10, 99, 20, 30], &[20, 30, 88], &[1, 2, 3, 4], &dense);
        assert_eq!(src, vec![0]);
        assert_eq!(dst, vec![1]);
        assert_eq!(rel, vec![1]);
    }
}
