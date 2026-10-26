fn main() {
    println!("cargo:rerun-if-changed=../../assets/themes");
    println!("cargo:rerun-if-changed=../../assets/icons");
}
