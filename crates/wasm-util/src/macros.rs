#[macro_export]
macro_rules! info {
    ( $( $t:tt )* ) => {
        $crate::__reexport::console::info_1(&format!( $( $t )* ).into());
    }
}

#[macro_export]
macro_rules! error {
    ( $( $t:tt )* ) => {
        $crate::__reexport::console::error_1(&format!( $( $t )* ).into());
    }
}

#[macro_export]
macro_rules! alert {
    ( $( $t:tt)* ) => {
        $crate::util::get_window().unwrap_throw()
            .alert_with_message(&format!( $( $t )* ))
            .unwrap();
    };
}
