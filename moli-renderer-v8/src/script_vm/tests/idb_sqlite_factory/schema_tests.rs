use super::*;

#[tokio::test]
async fn idb_sqlite_schema_validates_key_paths_synchronously_without_poisoning_upgrade() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let output = run(&mut page, &loader, r#"
        const errors = [];
        const check = (expected, action) => {
            let actual = 'no exception';
            try { action(); } catch (error) { actual = error.name; }
            if (actual !== expected) throw Error(`${expected}: got ${actual}`);
            errors.push(actual);
        };
        const db = await open('schema', 1, db => {
            const store = db.createObjectStore('records', {keyPath:'id'});
            store.createIndex('group','group');
            for (const keyPath of ['j a', 'a..b', []]) {
                check('SyntaxError', () => db.createObjectStore('invalid', {keyPath}));
                check('SyntaxError', () => store.createIndex('invalid', keyPath));
            }
            // createObjectStore checks syntax before duplicate names; createIndex
            // checks duplicate names first. Both check InvalidAccess afterward.
            check('SyntaxError', () => db.createObjectStore('records', {keyPath:'j a'}));
            check('ConstraintError', () => store.createIndex('group', 'j a'));
            check('SyntaxError', () => db.createObjectStore('invalid', {keyPath:['j a'], autoIncrement:true}));
            check('SyntaxError', () => store.createIndex('invalid', ['j a'], {multiEntry:true}));
            check('ConstraintError', () => db.createObjectStore('records', {keyPath:['id'], autoIncrement:true}));
            check('ConstraintError', () => store.createIndex('group', ['id'], {multiEntry:true}));
            check('InvalidAccessError', () => db.createObjectStore('invalid', {keyPath:['id'], autoIncrement:true}));
            check('InvalidAccessError', () => store.createIndex('invalid', ['id'], {multiEntry:true}));
            store.put({id:1, group:'ok'});
        });
        check('InvalidStateError', () => db.createObjectStore('invalid', {keyPath:'j a', autoIncrement:true}));
        const store = db.transaction('records').objectStore('records');
        check('InvalidStateError', () => store.createIndex('invalid', ['j a'], {multiEntry:true}));
        const value = await result(store.index('group').get('ok'));
        const names = [...db.objectStoreNames];
        db.close();
        return {errors, value, names};
    "#).await;
    assert_eq!(
        output,
        r#"{"errors":["SyntaxError","SyntaxError","SyntaxError","SyntaxError","SyntaxError","SyntaxError","SyntaxError","ConstraintError","SyntaxError","SyntaxError","ConstraintError","ConstraintError","InvalidAccessError","InvalidAccessError","InvalidStateError","InvalidStateError"],"value":{"id":1,"group":"ok"},"names":["records"]}"#
    );
}
