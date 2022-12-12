//! From "Visualizing Hyperbolic Space: Unusual Uses of 4x4 Matrices." Phillips, Gunn.
//!
//! Vector4 values are assumed to be homogeneous Klein model coordinates unless otherwise
//! stated. Note that Minkowski model coordinates are valid Klein coordinates, but not vis versa.

use na::{RealField, Scalar};
use serde::{Deserialize, Serialize};

/// A point on the surface of the 3D hyperboloid in Minkowski coordinates with an implicit w
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[repr(C)]
pub struct HPoint<N: Scalar>(na::Vector3<N>);

impl<N: Scalar> HPoint<N> {
    pub fn new(x: N, y: N, z: N) -> Self {
        Self(na::Vector3::new(x, y, z))
    }

    /// Construct from Minkowski coordinates
    pub fn from_homogeneous(v: &na::Vector4<N>) -> Self {
        Self(v.xyz())
    }
}

impl<N: RealField + Copy> HPoint<N> {
    pub fn origin() -> Self {
        Self::new(na::zero(), na::zero(), na::zero())
    }

    /// Convert to Minkowski coordinates
    pub fn to_homogeneous(self) -> na::Vector4<N> {
        // x^2 + y^2 + z^2 - w^2 = -1
        // sqrt(x^2 + y^2 + z^2 + 1) = w
        let w = (self.0.x.powi(2) + self.0.y.powi(2) + self.0.z.powi(2) + na::one()).sqrt();
        na::Vector4::new(self.0.x, self.0.y, self.0.z, w)
    }
}

/// Point or plane reflection around point or normal `p`
pub fn reflect<N: RealField + Copy>(p: &na::Vector4<N>) -> na::Matrix4<N> {
    na::Matrix4::<N>::identity()
        - minkowski_outer_product(p, p) * na::convert::<_, N>(2.0) / mip(p, p)
}

/// Transform that translates `a` to `b` given that `a` and `b` are Lorentz normalized pointlike vectors
pub fn translate<N: RealField + Copy>(a: &na::Vector4<N>, b: &na::Vector4<N>) -> na::Matrix4<N> {
    let a_plus_b = a + b;
    na::Matrix4::<N>::identity() - minkowski_outer_product(b, a) * na::convert::<_, N>(2.0)
        + minkowski_outer_product(&a_plus_b, &a_plus_b) / (N::one() - mip(a, b))
}

/// Transform that translates the origin in the direction of the given vector with distance equal to its magnitude
pub fn translate_along<N: RealField + Copy>(v: &na::Vector3<N>) -> na::Matrix4<N> {
    let norm = v.norm();
    if norm == na::zero() {
        return na::Matrix4::identity();
    }
    // g = Lorentz gamma factor
    let g = norm.cosh();
    let bgc = norm.sinhc();
    translate(&origin(), &(v * bgc).insert_row(3, g))
}

/// 4D reflection around a normal vector; length is not significant (so long as it's nonzero)
pub fn euclidean_reflect<N: RealField + Copy>(v: &na::Vector4<N>) -> na::Matrix4<N> {
    na::Matrix4::identity() - v * v.transpose() * (na::convert::<_, N>(2.0) / v.norm_squared())
}

pub fn midpoint<N: RealField + Copy>(a: &na::Vector4<N>, b: &na::Vector4<N>) -> na::Vector4<N> {
    a * (mip(b, b) * mip(a, b)).sqrt() + b * (mip(a, a) * mip(a, b)).sqrt()
}

pub fn distance<N: RealField + Copy>(a: &na::Vector4<N>, b: &na::Vector4<N>) -> N {
    (mip(a, b).powi(2) / (mip(a, a) * mip(b, b))).sqrt().acosh()
}

pub fn origin<N: RealField + Copy>() -> na::Vector4<N> {
    na::Vector4::new(na::zero(), na::zero(), na::zero(), na::one())
}

pub fn lorentz_normalize<N: RealField + Copy>(v: &na::Vector4<N>) -> na::Vector4<N> {
    let sf2 = mip(v, v);
    if sf2 == na::zero() {
        return origin();
    }
    let sf = sf2.abs().sqrt();
    v / sf
}

pub fn renormalize_isometry<N: RealField + Copy>(m: &na::Matrix4<N>) -> na::Matrix4<N> {
    let boost = translate(&origin(), &m.index((.., 3)).into());
    let inverse_boost = mtranspose(&boost);
    let rotation = renormalize_rotation_reflection(
        &(inverse_boost * m).fixed_slice::<3, 3>(0, 0).clone_owned(),
    );
    boost * rotation.to_homogeneous()
}

#[rustfmt::skip]
fn renormalize_rotation_reflection<N: RealField + Copy>(m: &na::Matrix3<N>) -> na::Matrix3<N> {
    let zv = m.index((.., 2)).normalize();
    let yv = m.index((.., 1));
    let dot = zv.dot(&yv);
    let yv = na::Vector3::new(yv.x - dot * zv.x, yv.y - dot * zv.y, yv.z - dot * zv.z).normalize();
    let sign = m.determinant().signum();
    na::Matrix3::new(
        sign * (yv.y * zv.z - yv.z * zv.y), yv.x, zv.x,
        sign * (yv.z * zv.x - yv.x * zv.z), yv.y, zv.y,
        sign * (yv.x * zv.y - yv.y * zv.x), yv.z, zv.z,
    )
}

/// Whether an isometry reverses winding with respect to the norm
pub fn parity<N: RealField + Copy>(m: &na::Matrix4<N>) -> bool {
    m.fixed_slice::<3, 3>(0, 0).determinant() < na::zero::<N>()
}

/// Minkowski inner product, aka <a, b>_h
pub fn mip<N: RealField + Copy>(a: &na::Vector4<N>, b: &na::Vector4<N>) -> N {
    a.x * b.x + a.y * b.y + a.z * b.z - a.w * b.w
}

/// Minkowski transpose. Inverse for hyperbolic isometries
#[rustfmt::skip]
pub fn mtranspose<N: RealField + Copy>(m: &na::Matrix4<N>) -> na::Matrix4<N> {
    na::Matrix4::new(
         m.m11,  m.m21,  m.m31, -m.m41,
         m.m12,  m.m22,  m.m32, -m.m42,
         m.m13,  m.m23,  m.m33, -m.m43,
        -m.m14, -m.m24, -m.m34,  m.m44,
    )
}

fn minkowski_outer_product<N: RealField + Copy>(
    a: &na::Vector4<N>,
    b: &na::Vector4<N>,
) -> na::Matrix4<N> {
    *a * na::RowVector4::new(b.x, b.y, b.z, -b.w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::*;

    #[test]
    #[rustfmt::skip]
    fn reflect_example() {
        assert_abs_diff_eq!(
            reflect(&lorentz_normalize(&na::Vector4::new(0.5, 0.0, 0.0, 1.0))),
            na::Matrix4::new(
                1.666, 0.0, 0.0, -1.333,
                0.0  , 1.0, 0.0,  0.0,
                0.0  , 0.0, 1.0,  0.0,
                1.333, 0.0, 0.0, -1.666
            ),
            epsilon = 1e-3
        );
    }

    #[test]
    #[rustfmt::skip]
    fn translate_example() {
        assert_abs_diff_eq!(
            translate(
                &lorentz_normalize(&na::Vector4::new(-0.5, -0.5, 0.0, 1.0)),
                &lorentz_normalize(&na::Vector4::new(0.3, -0.7, 0.0, 1.0))
            ),
            na::Matrix4::new(
                 1.676, 0.814, 0.0,  1.572,
                -1.369, 0.636, 0.0, -1.130,
                 0.0,   0.0,   1.0,  0.0,
                 1.919, 0.257, 0.0,  2.179,
            ),
            epsilon = 1e-3
        );
    }

    #[test]
    fn translate_identity() {
        let a = lorentz_normalize(&na::Vector4::new(-0.5, -0.5, 0.0, 1.0));
        let b = lorentz_normalize(&na::Vector4::new(0.3, -0.7, 0.0, 1.0));
        let o = na::Vector4::new(0.0, 0.0, 0.0, 1.0);
        assert_abs_diff_eq!(
            translate(&a, &b),
            translate(&o, &a) * translate(&o, &(translate(&a, &o) * b)) * translate(&a, &o),
            epsilon = 1e-5
        );
    }

    #[test]
    fn translate_equivalence() {
        let a = lorentz_normalize(&na::Vector4::new(-0.5, -0.5, 0.0, 1.0));
        let o = na::Vector4::new(0.0, 0.0, 0.0, 1.0);
        let direction = a.xyz().normalize();
        let distance = dbg!(distance(&o, &a));
        assert_abs_diff_eq!(
            translate(&o, &a),
            translate_along(&(direction * distance)),
            epsilon = 1e-5
        );
    }

    #[test]
    fn translate_distance() {
        let dx = 2.3;
        let xf = translate_along(&(na::Vector3::x() * dx));
        assert_abs_diff_eq!(dx, distance(&origin(), &(xf * origin())));
    }

    #[test]
    fn distance_example() {
        let a = na::Vector4::new(0.2, 0.0, 0.0, 1.0);
        let b = na::Vector4::new(-0.5, -0.5, 0.0, 1.0);
        // Paper doubles distances for reasons unknown
        assert_abs_diff_eq!(distance(&a, &b), 2.074 / 2.0, epsilon = 1e-3);
    }

    #[test]
    fn distance_commutative() {
        let p = HPoint::new(-1.0, -1.0, 0.0).to_homogeneous();
        let q = HPoint::new(1.0, -1.0, 0.0).to_homogeneous();
        assert_abs_diff_eq!(distance(&p, &q), distance(&q, &p));
    }

    #[test]
    fn midpoint_distance() {
        let p = HPoint::new(-1.0, -1.0, 0.0).to_homogeneous();
        let q = HPoint::new(1.0, -1.0, 0.0).to_homogeneous();
        let m = midpoint(&p, &q);
        assert_abs_diff_eq!(distance(&p, &m), distance(&m, &q), epsilon = 1e-5);
        assert_abs_diff_eq!(distance(&p, &m) * 2.0, distance(&p, &q), epsilon = 1e-5);
    }

    #[test]
    fn renormalize_translation() {
        let mat = translate(
            &lorentz_normalize(&na::Vector4::new(-0.5, -0.5, 0.0, 1.0)),
            &lorentz_normalize(&na::Vector4::new(0.3, -0.7, 0.0, 1.0)),
        );
        assert_abs_diff_eq!(renormalize_isometry(&mat), mat, epsilon = 1e-5);
    }

    #[test]
    #[rustfmt::skip]
    fn renormalize_reflection() {
        let mat = na::Matrix4::new(-1.0, 0.0, 0.0, 0.0,
                                   0.0, 1.0, 0.0, 0.0,
                                   0.0, 0.0, 1.0, 0.0,
                                   0.0, 0.0, 0.0, 1.0);
        assert_abs_diff_eq!(renormalize_isometry(&mat), mat, epsilon = 1e-5);
    }
}
