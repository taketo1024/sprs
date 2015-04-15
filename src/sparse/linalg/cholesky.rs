/// Cholesky factorization

use std::ops::{Deref};

use num::traits::Num;

use sparse::csmat::{CsMat, CompressedStorage};
use sparse::symmetric::{is_symmetric};
use sparse::permutation::Permutation;

pub enum SymmetryCheck {
    CheckSymmetry,
    DontCheckSymmetry
}

/// Perform a symbolic LDLt decomposition of a symmetric sparse matrix
pub fn ldl_symbolic<N, IStorage, DStorage, PStorage>(
    mat: &CsMat<N, IStorage, DStorage>,
    perm: &Permutation<PStorage>,
    l_colptr: &mut [usize],
    parents: &mut [isize],
    l_nz: &mut [usize],
    flag_workspace: &mut [usize],
    check_symmetry: SymmetryCheck)
where
N: Clone + Copy + PartialEq,
IStorage: Deref<Target=[usize]>,
DStorage: Deref<Target=[N]>,
PStorage: Deref<Target=[usize]> {

    match check_symmetry {
        SymmetryCheck::DontCheckSymmetry => (),
        SymmetryCheck::CheckSymmetry => if ! is_symmetric(mat) {
            panic!("Matrix is not symmetric")
        }
    }

    let n = mat.rows();

    for (k, (outer_ind, vec)) in mat.outer_iterator_papt(&perm.borrowed()).enumerate() {

        flag_workspace[k] = k; // this node is visited
        parents[k] = -1;
        l_nz[k] = 0;

        for (inner_ind, _) in vec.iter() {
            let mut i = inner_ind;

            // FIXME: the article tests inner_ind versus k, but this looks
            // weird as it would introduce a dissimetry between the permuted
            // and non permuted cases. Needs test however
            if i < outer_ind {
                // get back to the root of the etree
                // TODO: maybe this calls for a more adequate parent structure?
                while flag_workspace[i] != outer_ind {
                    if parents[i] == -1 {
                        parents[i] = outer_ind as isize; // TODO check overflow
                    }
                    l_nz[i] = l_nz[i] + 1;
                    flag_workspace[i] = outer_ind;
                    i = parents[i] as usize; // TODO check negative
                }
            }
        }
    }

    let mut prev : usize = 0;
    for (k, colptr) in (0..n).zip(l_colptr.iter_mut()) {
        *colptr = prev;
        prev += l_nz[k];
    }
    l_colptr[n] = prev;

}

pub fn ldl_numeric<N, IStorage, DStorage, PStorage>(
    mat: &CsMat<N, IStorage, DStorage>,
    l_colptr: &[usize],
    parents: &[isize],
    perm: &Permutation<PStorage>,
    l_nz: &mut [usize],
    l_indices: &mut [usize],
    l_data: &mut [N],
    diag: &mut [N],
    y_workspace: &mut [N],
    pattern_workspace: &mut [usize],
    flag_workspace: &mut [usize])
where
N: Clone + Copy + PartialEq + Num + PartialOrd,
IStorage: Deref<Target=[usize]>,
DStorage: Deref<Target=[N]>,
PStorage: Deref<Target=[usize]> {

    let n = mat.rows();

    for (k, (outer_ind, vec))
    in mat.outer_iterator_papt(&perm.borrowed()).enumerate() {

        // compute the nonzero pattern of the kth row of L
        // in topological order

        flag_workspace[k] = k; // this node is visited
        y_workspace[k] = N::zero();
        l_nz[k] = 0;
        let mut top = n;

        for (inner_ind, val) in vec.iter().filter(|&(i,_)| i <= k) {
            y_workspace[inner_ind] = y_workspace[inner_ind] + val;
            let mut i = inner_ind;
            let mut len = 0;
            while flag_workspace[i] != outer_ind {
                pattern_workspace[len] = i;
                len += 1;
                flag_workspace[i] = k;
                i = parents[i] as usize;
            }
            while len > 0 { // TODO: can be written as a loop with iterators
                top -= 1;
                len -= 1;
                pattern_workspace[top] = pattern_workspace[len];
            }
        }

        // use a sparse triangular solve to compute the values
        // of the kth row of L
        diag[k] = y_workspace[k];
        y_workspace[k] = N::zero();
        'pattern: for &i in &pattern_workspace[top..n] {
            let yi = y_workspace[i];
            y_workspace[i] = N::zero();
            let p2 = l_colptr[i] + l_nz[i];
            for p in l_colptr[i]..p2 {
                // we cannot go inside this loop before something has actually
                // be written into l_indices[l_colptr[i]..p2] so this
                // read is actually not into garbage
                // actually each iteration of the 'pattern loop adds writes the
                // value in l_indices that will be read on the next iteration
                // TODO: can some design change make this fact more obvious?
                let y_index = l_indices[p];
                y_workspace[y_index] = y_workspace[y_index] - l_data[p] * yi;
            }
            let l_ki = yi / diag[i];
            diag[k] = diag[k] - l_ki * yi;
            l_indices[p2] = k;
            l_data[p2] = l_ki;
            l_nz[i] += 1;
        }
        if diag[k] == N::zero() {
            panic!("Matrix is singular");
        }
    }
}

pub fn ldl_lsolve<N>(
    l_colptr: &[usize],
    l_indices: &[usize],
    l_data: &[N],
    x: &mut [N])
where
N: Clone + Copy + Num {

    let n = l_colptr.len() - 1;
    let l = CsMat::from_slices(
        CompressedStorage::CSC, n, n, l_colptr, l_indices, l_data).unwrap();
    for (col_ind, vec) in l.outer_iterator() {
        for (row_ind, value) in vec.iter() {
            x[row_ind] = x[row_ind] - value * x[col_ind];
        }
    }
}

pub fn ldl_ltsolve<N>(
    l_colptr: &[usize],
    l_indices: &[usize],
    l_data: &[N],
    x: &mut [N])
where
N: Clone + Copy + Num {
    // the ltsolve is a very specific iteration on the matrix, we're iterating
    // the outer dimension in reverse but the inner dimension in the usual way
    // It might make sense to abstract it later if it turns out to be
    // a common pattern, but we're better of doing it by hand here for now
    for (outer_ind, inner_window) in l_colptr.windows(2).enumerate().rev() {
        let start = inner_window[0];
        let end = inner_window[1];
        for (&inner_ind, &val)
                in l_indices[start..end].iter().zip(l_data[start..end].iter()) {
            x[outer_ind] = x[outer_ind] - val * x[inner_ind];
        }
    }
}

pub fn ldl_dsolve<N>(
    d: &[N],
    x: &mut [N])
where
N: Clone + Copy + Num {

    for (xv, dv) in x.iter_mut().zip(d.iter()) {
        *xv = *xv / *dv;
    }
}

#[cfg(test)]
mod test {
    use sparse::csmat::CsMat;
    use sparse::csmat::CompressedStorage::{CSC};
    use sparse::permutation::Permutation;
    use super::{SymmetryCheck};

    fn test_mat1() -> CsMat<f64, Vec<usize>, Vec<f64>> {
        let indptr = vec![0, 2, 5, 6, 7, 13, 14, 17, 20, 24, 28];
        let indices = vec![
            0, 8,
            1, 4, 9,
            2,
            3,
            1, 4, 6, 7, 8, 9,
            5,
            4, 6, 9,
            4, 7, 8,
            0, 4, 7, 8,
            1, 4, 6, 9];
        let data = vec![
            1.7, 0.13,
            1., 0.02, 0.01,
            1.5,
            1.1,
            0.02, 2.6, 0.16, 0.09, 0.52, 0.53,
            1.2,
            0.16, 1.3, 0.56,
            0.09, 1.6, 0.11,
            0.13, 0.52, 0.11, 1.4,
            0.01, 0.53, 0.56, 3.1];
        CsMat::from_vecs(CSC, 10, 10, indptr, indices, data).unwrap()
    }

    fn test_vec1() -> Vec<f64> {
        vec![0.287, 0.22, 0.45, 0.44, 2.486, 0.72 ,
             1.55 ,  1.424,1.621,  3.759]
    }

    fn expected_factors1() -> (Vec<usize>, Vec<usize>, Vec<f64>, Vec<f64>) {
        let expected_lp = vec![0, 1, 3, 3, 3, 7, 7, 10, 12, 13, 13];
        let expected_li = vec![8, 4, 9, 6, 7, 8, 9, 7, 8, 9, 8, 9, 9];
        let expected_lx = vec![
            0.076470588235294124, 0.02, 0.01, 0.061547930450838589,
            0.034620710878596701, 0.20003077396522542, 0.20380058470533929,
            -0.0042935346524025902, -0.024807089102770519,
            0.40878266366119237, 0.05752526570865537,
            -0.010068305077340346, -0.071852278207562709];
        let expected_d = vec![
            1.7, 1., 1.5, 1.1000000000000001, 2.5996000000000001, 1.2,
            1.290152331127866, 1.5968603527854308, 1.2799646117414738,
            2.7695677698030283];
        (expected_lp, expected_li, expected_lx, expected_d)
    }

    fn expected_lsolve_res1() -> Vec<f64> {
        vec![0.28699999999999998, 0.22, 0.45000000000000001, 0.44,
             2.4816000000000003, 0.71999999999999997, 1.3972626557931991,
             1.3440844395148306, 1.0599997771886431, 2.7695677698030279]
    }

    fn expected_dsolve_res1() -> Vec<f64> {
        vec![0.16882352941176471, 0.22, 0.29999999999999999,
             0.39999999999999997, 0.95460840129250657, 0.59999999999999998,
             1.0830214557467768, 0.84170443406044937, 0.82814772179243734,
             0.99999999999999989]
    }

    fn expected_res1() -> Vec<f64> {
        vec![0.099999999999999992, 0.19999999999999998,
             0.29999999999999999, 0.39999999999999997,
             0.5, 0.59999999999999998,
             0.70000000000000007, 0.79999999999999993,
             0.90000000000000002, 0.99999999999999989]
    }

    #[test]
    fn test_factor1() {
        let mut l_colptr = [0; 11];
        let mut parents = [0; 10];
        let mut l_nz = [0; 10];
        let mut flag_workspace = [0; 10];
        let perm : Permutation<&[usize]> = Permutation::identity();
        let mat = test_mat1();
        super::ldl_symbolic(&mat, &perm, &mut l_colptr, &mut parents,
                            &mut l_nz, &mut flag_workspace,
                            SymmetryCheck::CheckSymmetry);

        let nnz = l_colptr[10];
        let mut l_indices = vec![0; nnz];
        let mut l_data = vec![0.; nnz];
        let mut diag = [0.; 10];
        let mut y_workspace = [0.; 10];
        let mut pattern_workspace = [0; 10];
        super::ldl_numeric(&mat, &l_colptr, &parents, &perm, &mut l_nz,
                           &mut l_indices, &mut l_data, &mut diag,
                           &mut y_workspace, &mut pattern_workspace,
                           &mut flag_workspace);

        let (expected_lp, expected_li, expected_lx, expected_d) = expected_factors1();

        assert_eq!(&l_colptr, &expected_lp[..]);
        assert_eq!(&l_indices, &expected_li);
        assert_eq!(&l_data, &expected_lx);
        assert_eq!(&diag, &expected_d[..]);
    }

    #[test]
    fn test_solve1() {
        let (expected_lp, expected_li, expected_lx, expected_d) = expected_factors1();
        let b = test_vec1();
        let mut x = b.clone();
        super::ldl_lsolve(&expected_lp, &expected_li, &expected_lx, &mut x);
        assert_eq!(&x, &expected_lsolve_res1());
        super::ldl_dsolve(&expected_d, &mut x);
        assert_eq!(&x, &expected_dsolve_res1());
        super::ldl_ltsolve(&expected_lp, &expected_li, &expected_lx, &mut x);

        let x0 = expected_res1();
        assert_eq!(x, x0);
    }

    #[test]
    fn test_factor_solve1() {
        // FIXME: do better when compile time ints available..
        // eg with
        // let n = 10;
        let mut l_colptr = [0; 11];
        let mut parents = [0; 10];
        let mut l_nz = [0; 10];
        let mut flag_workspace = [0; 10];
        let perm : Permutation<&[usize]> = Permutation::identity();
        let mat = test_mat1();
        super::ldl_symbolic(&mat, &perm, &mut l_colptr, &mut parents,
                            &mut l_nz, &mut flag_workspace,
                            SymmetryCheck::CheckSymmetry);

        let nnz = l_colptr[10];
        let mut l_indices = vec![0; nnz];
        let mut l_data = vec![0.; nnz];
        let mut diag = [0.; 10];
        let mut y_workspace = [0.; 10];
        let mut pattern_workspace = [0; 10];
        super::ldl_numeric(&mat, &l_colptr, &parents, &perm, &mut l_nz,
                           &mut l_indices, &mut l_data, &mut diag,
                           &mut y_workspace, &mut pattern_workspace,
                           &mut flag_workspace);

        let b = test_vec1();
        let mut x = b.clone();
        super::ldl_lsolve(&l_colptr, &l_indices, &l_data, &mut x);
        super::ldl_dsolve(&diag, &mut x);
        super::ldl_ltsolve(&l_colptr, &l_indices, &l_data, &mut x);

        let x0 = expected_res1();
        assert_eq!(x, x0);
    }
}
