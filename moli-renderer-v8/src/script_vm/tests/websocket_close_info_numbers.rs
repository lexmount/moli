use super::*;

#[test]
fn websocket_error_close_info_truncates_fractional_codes_before_validation() {
    let mut vm = new_parsed_test_vm(
        "https://close-info-numbers.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"JSON.stringify([3000.4, 3000.5, 3001.5, 1000.5].map(closeCode => new WebSocketError('', {closeCode}).closeCode))"#).unwrap(), "[3000,3000,3001,1000]");
    assert_eq!(vm.eval(r#"JSON.stringify([999.5, 68536, -62536, 4294970296, NaN, Infinity, -Infinity].map(closeCode => {
        try { new WebSocketError('', {closeCode}); return 'accepted'; }
        catch (error) { return error.name; }
    }))"#).unwrap(), serde_json::to_string(&["InvalidAccessError"; 7]).unwrap());
}
