// Execute the real patched controller against a repository double; no Mac writes.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const ts = require('typescript');
const vm = require('node:vm');
const root = process.argv[2];
const path = '/packages/server/src/server/api/http/api/v1/';
const source = fs.readFileSync(root + path + 'routers/contactRouter.ts', 'utf8');
const transpiled = ts.transpileModule(source, {compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020},reportDiagnostics:true});
assert.equal(transpiled.diagnostics.length,0);
let stored = {id:7,firstName:'Alex',lastName:'Morgan',displayName:'Old name',avatar:'unchanged-avatar',phoneNumbers:['+15551234567'],emails:['a@example.com']};
let writes = 0;
const contacts = {
    findDbContact: async ({contactId}) => {assert.equal(contactId,7); return {...stored};},
    createContact: async fields => {
        writes++;
        if (fields.updateEntry) {
            assert.equal(fields.id,7);
            assert.equal(fields.firstName,'Alex');assert.equal(fields.lastName,'Morgan');
            assert.equal(fields.avatar,undefined);assert.equal(fields.phoneNumbers,undefined);
            stored={...stored,displayName:fields.displayName};return stored;
        }
        assert.equal(fields.updateEntry,false);
        return {id:8,...fields};
    },
    getAllContacts: async () => [stored],
    mapContacts: rows => rows.map(row=>({...row,sourceType:'db'})),
};
const exportsObject = {};
vm.runInNewContext(transpiled.outputText,{exports:exportsObject,require(name){
    if(name.endsWith('contactInterface'))return {ContactInterface:contacts};
    if(name.endsWith('/success'))return {Success:class {constructor(ctx,data){this.data=data;}send(){return this.data;}}};
    if(name.endsWith('/errors'))return {BadRequest:class extends Error{}};
    return {};
},console});
(async()=>{
    const Router=exportsObject.ContactRouter;
    const capability=await Router.capabilities({},null);assert.equal(capability.data.localContactNames,true);
    const result=await Router.updateName({params:{id:'7'},request:{body:{displayName:'New name'}}},null);
    assert.equal(result.data.id,7);assert.equal(result.data.displayName,'New name');
    assert.equal(result.data.avatar,'unchanged-avatar');assert.deepEqual(result.data.phoneNumbers,['+15551234567']);
    for(const id of ['0','-1','7oops','9007199254740993'])await assert.rejects(()=>Router.updateName({params:{id},request:{body:{displayName:'Bad'}}},null));
    for(const displayName of ['',null,'x'.repeat(201)])await assert.rejects(()=>Router.updateName({params:{id:'7'},request:{body:{displayName}}},null));
    await assert.rejects(()=>Router.createLocal({request:{body:{displayName:'Duplicate',address:'+1 (555) 123-4567'}}},null));
    assert.equal(writes,1);
    const created=await Router.createLocal({request:{body:{displayName:'New contact',address:'new@example.com'}}},null);
    assert.equal(created.data.id,8);assert.equal(created.data.emails[0],'new@example.com');
    const routes=fs.readFileSync(root+path+'httpRoutes.ts','utf8');
    const group=routes.slice(routes.indexOf('name: "Contact"'),routes.indexOf('name: "Backup"'));
    assert.match(group,/middleware: HttpRoutes.protected/);
    for(const controller of ['capabilities','updateName','createLocal'])assert.ok(group.includes('ContactRouter.'+controller));
    console.log('Server contact patch: exact-ID edits, preserved fields, validation, duplicate rejection, create, and protected routes passed');
})().catch(error=>{console.error(error);process.exitCode=1;});
