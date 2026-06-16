#[cfg(test)]
mod zero_ray_bug_analysis {
    use crate::aabb::Aabb;
    use crate::math::Vec3;
    use crate::ray::Ray;

    #[test]
    fn zero_y_component_creates_infinite_inv_dir() {
        // Ray with zero y-component (vertical plane in y)
        let dir = Vec3::new(1.0, 0.0, 1.0);
        let ray = Ray::new(Vec3::new(0.0, 5.0, 0.0), dir);

        // inv_dir.y should be infinity
        assert!(ray.inv_dir.y.is_infinite(), "inv_dir.y should be infinite");
        
        println!("inv_dir = ({}, {}, {})", ray.inv_dir.x, ray.inv_dir.y, ray.inv_dir.z);
    }

    #[test]
    fn zero_direction_causes_aabb_miss_due_to_nan() {
        // Ray with zero y-velocity starting above the box
        let origin = Vec3::new(0.0, 5.0, 0.0);
        let dir = Vec3::new(1.0, 0.0, 1.0);  // zero y component
        let ray = Ray::new(origin, dir);

        // AABB centered at origin, unit size
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        // Ray starts at y=5 with zero y-velocity, so it should miss the y-range [-1, 1]
        // But let's see what happens in the slab test
        let t0 = (aabb.min - origin).mul(ray.inv_dir);
        let t1 = (aabb.max - origin).mul(ray.inv_dir);

        println!("t0 = ({}, {}, {})", t0.x, t0.y, t0.z);
        println!("t1 = ({}, {}, {})", t1.x, t1.y, t1.z);

        // The y-component will be:
        // t0.y = (min.y - origin.y) * inv_dir.y = (-1.0 - 5.0) * inf = -inf
        // t1.y = (max.y - origin.y) * inv_dir.y = (1.0 - 5.0) * inf = -inf
        // Both should be negative infinity
        
        let hit = aabb.hit(&ray, f32::INFINITY);
        println!("Ray at (0, 5, 0) dir (1, 0, 1) hits AABB: {}", hit);
        // This should be false, because the ray misses the y-range
    }

    #[test]
    fn zero_mult_inf_produces_nan() {
        // (min.x - origin.x) * inv_dir.x when:
        // - min.x == origin.x (distance is 0)
        // - inv_dir.x == inf (direction.x == 0)
        // This produces 0 * inf = NaN
        
        let origin = Vec3::new(0.0, 0.0, 0.0);
        let dir = Vec3::new(0.0, 1.0, 0.0);  // zero x and z
        let ray = Ray::new(origin, dir);

        let aabb = Aabb::new(Vec3::new(0.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        
        let t0 = (aabb.min - origin).mul(ray.inv_dir);
        println!("t0 = ({}, {}, {})", t0.x, t0.y, t0.z);
        println!("t0.x is NaN: {}", t0.x.is_nan());
        
        // If any component is NaN, max_component() will propagate it
        let tmin = t0.min(Vec3::splat(f32::INFINITY));
        println!("max_component of result with NaN: {}", tmin.max_component());
    }

    #[test]
    fn nan_in_comparison_is_always_false() {
        let nan = f32::NAN;
        assert!(!( nan <= 0.0 ), "NaN <= x is always false");
        assert!(!( nan >= 0.0 ), "NaN >= x is always false");
        assert!(!( nan == nan ), "NaN == NaN is false");
        
        // This means if enter or exit contains NaN, the comparison will fail
        // even if it should logically succeed
    }

    #[test]
    fn multiple_zero_components() {
        // Ray with two zero components: (0, 0, 1)
        let origin = Vec3::new(5.0, 5.0, 0.0);
        let dir = Vec3::new(0.0, 0.0, 1.0);
        let ray = Ray::new(origin, dir);

        assert!(ray.inv_dir.x.is_infinite());
        assert!(ray.inv_dir.y.is_infinite());
        assert!(!ray.inv_dir.z.is_infinite());

        // AABB at origin
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        // The ray starts at (5, 5) in the x-y plane
        // Directions: x=0 (never changes), y=0 (never changes)
        // So it can never reach x in [-1, 1] or y in [-1, 1]
        // It should MISS
        
        let hit = aabb.hit(&ray, f32::INFINITY);
        println!("Ray at (5, 5, 0) dir (0, 0, 1) hits AABB: {}", hit);
        // This should be false
    }
}
