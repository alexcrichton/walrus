#[no_mangle]
pub extern fn foo() {
    bar::bar();
}

mod bar;
