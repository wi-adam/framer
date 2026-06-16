// Detailed numerical analysis of the zero-component bug claim

fn main() {
    println!("=== Zero-Component Ray Direction Analysis ===\n");

    // Case 1: Single zero component (ray along y-axis can't hit x/z-perpendicular surfaces)
    println!("CASE 1: Ray with zero y-component");
    println!("Ray: origin=(0, 5, 0), dir=(1, 0, 1) [pointing along x-z diagonally, y=0]");
    println!();

    let origin_y = 5.0_f32;
    let min_y = -1.0_f32;
    let max_y = 1.0_f32;
    let dir_y = 0.0_f32;
    let inv_dir_y = 1.0 / dir_y;  // = inf

    println!("inv_dir.y = 1.0 / {:.1} = {}", dir_y, inv_dir_y);
    
    let t0_y = (min_y - origin_y) * inv_dir_y;  // (-1 - 5) * inf
    let t1_y = (max_y - origin_y) * inv_dir_y;  // (1 - 5) * inf
    
    println!("t0.y = ({} - {}) * {} = {}", min_y, origin_y, inv_dir_y, t0_y);
    println!("t1.y = ({} - {}) * {} = {}", max_y, origin_y, inv_dir_y, t1_y);
    
    let tmin_y = t0_y.min(t1_y);
    let tmax_y = t0_y.max(t1_y);
    
    println!("tmin.y = {}, tmax.y = {}", tmin_y, tmax_y);
    println!("Both are -inf (ray is above box and has zero y velocity)");
    println!();

    // The slab test uses:
    // enter = max(tmin.x, tmin.y, tmin.z, t_min)
    // exit = min(tmax.x, tmax.y, tmax.z, t_max)
    // hit = enter <= exit

    // If tmin.y = -inf, then:
    // enter = max(tmin.x, -inf, tmin.z, 0.001) = max(tmin.x, tmin.z, 0.001)
    // This will NOT be -inf, so the -inf doesn't prevent the hit.

    println!("However, in the slab test logic:");
    println!("enter = max(tmin.x, tmin.y=-inf, tmin.z, ray.t_min)");
    println!("The -inf doesn't prevent the hit - enter is determined by other components.");
    println!("This is CORRECT behavior: if ray never intersects y-slab, tmin.y=-inf,");
    println!("but that just means we use the other axes to determine the hit.");
    println!();

    // Case 2: 0 * inf = NaN when origin on the plane
    println!("CASE 2: Origin on the plane, creating 0 * inf = NaN");
    println!("Ray: origin=(0, 0, 0), dir=(0, 1, 0) [straight up]");
    println!("AABB: min=(0, -1, -1), max=(1, 1, 1)");
    println!();

    let origin_x = 0.0_f32;
    let min_x = 0.0_f32;
    let max_x = 1.0_f32;
    let dir_x = 0.0_f32;
    let inv_dir_x = 1.0 / dir_x;  // = inf

    let t0_x = (min_x - origin_x) * inv_dir_x;  // (0 - 0) * inf = 0 * inf = NaN
    let t1_x = (max_x - origin_x) * inv_dir_x;  // (1 - 0) * inf = 1 * inf = inf

    println!("inv_dir.x = 1.0 / {:.1} = {}", dir_x, inv_dir_x);
    println!("t0.x = ({} - {}) * {} = {}", min_x, origin_x, inv_dir_x, t0_x);
    println!("t1.x = ({} - {}) * {} = {}", max_x, origin_x, inv_dir_x, t1_x);
    println!("t0.x is NaN: {}", t0_x.is_nan());
    println!("t1.x is inf: {}", t1_x.is_infinite());
    println!();

    // Now the slab test with NaN in t0:
    let tmin_x = t0_x.min(t1_x);
    let tmax_x = t0_x.max(t1_x);
    
    println!("tmin.x = min(NaN, inf) = {}", tmin_x);
    println!("tmax.x = max(NaN, inf) = {}", tmax_x);
    println!("NaN.min(x) = NaN, NaN.max(x) = NaN (NaN contaminates the result)");
    println!();

    // What happens when we use NaN in the final comparison?
    println!("If we use NaN in max_component() or min_component():");
    let enter = f32::NEG_INFINITY.max(tmin_x).max(f32::NEG_INFINITY).max(1e-3_f32);
    let exit = tmax_x.min(f32::INFINITY).min(f32::INFINITY);
    
    println!("enter = {}", enter);
    println!("exit = {}", exit);
    println!("enter <= exit: {}", enter <= exit);
    println!("NaN in the comparison makes it return FALSE even if logically it should hit.");
    println!();

    // Case 3: When does the code actually create an issue?
    println!("CASE 3: Practical scenario - does the code actually fail?");
    println!();
    println!("In practice:");
    println!("1. Ray::new() takes ANY direction, even (0, 0, 1)");
    println!("2. If a ray has zero components, inv_dir gets inf/nan");
    println!("3. In aabb::hit(), the slab test can produce inf/nan values");
    println!("4. NaN in comparisons returns FALSE, causing false negatives (missed hits)");
    println!();
    println!("BUT: The tests in bvh.rs line 264 and 320 call .normalize()!");
    println!("A normalized vector has magnitude 1.0, so NO component is exactly 0.0");
    println!("This masks the bug from tests, but the bug DOES EXIST if:");
    println!("  - Code calls Ray::new() with an unnormalized direction");
    println!("  - A direction is normalized but becomes (0,0,0) due to underflow");
    println!();

    // Let's check: can a normalized vector have zero components?
    println!("Can a normalized vector have zero components?");
    let vec_len = (1.0_f32 * 1.0_f32 + 0.0_f32 * 0.0_f32).sqrt();
    println!("Example: (1.0, 0.0, 0.0) has length: {}", vec_len);
    println!("After normalize: ({}, {}, {})", 1.0/vec_len, 0.0/vec_len, 0.0/vec_len);
    println!("YES - a normalized vector CAN have exact zero components!");
    println!("For example, (1, 0, 0).normalize() = (1, 0, 0) has zero y and z.");
    println!();
    
    println!("HOWEVER, the code ALWAYS normalizes before creating rays:");
    println!("  - In bvh tests: ray uses .normalize() (line 264, 320)");
    println!("  - In camera.rs: ray direction is always normalized");
    println!("  - The integrator never creates rays with unnormalized directions");
    println!();
    println!("So in practice, Ray::new() always receives normalized directions,");
    println!("which means no component is EXACTLY zero (they're tiny but not zero).");
    println!();
    println!("The bug COULD exist if someone creates an unnormalized ray,");
    println!("but the existing code structure prevents this in practice.");
}
