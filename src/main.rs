fn main() {
    #[cfg(not(target_os = "android"))]
    retrackt::app::run();
}
