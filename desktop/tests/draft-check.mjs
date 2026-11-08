import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {randomUUID} from 'node:crypto';
import vm from 'node:vm';
const source=readFileSync(new URL('../ui/app.js',import.meta.url),'utf8');
const storage=new Map();let fail=false;
const fresh=()=>{
  const context={crypto:{randomUUID},setTimeout,clearTimeout,console,
    localStorage:{getItem:k=>storage.get(k)??null,setItem:(k,v)=>{if(fail)throw Error('storage unavailable');storage.set(k,v);},removeItem:k=>storage.delete(k)},
    document:{addEventListener(){}},window:{addEventListener(){}},location:{hash:''}};
  vm.runInNewContext(source+';globalThis.store=MessageDrafts;',context);return context.store;
};
let S=fresh();S.open('appliance-a','alice');const a=S.create(),b=S.create();
a.text='First draft';a.files=[{uid:randomUUID(),id:'file-first',name:'notes.txt',media_type:'text/plain',size:12,state:'ready'}];b.text='Second draft';S.changed(true);
S=fresh();S.open('appliance-a','alice');assert.equal(S.all().length,2);assert.equal(S.get(a.id).text,'First draft');assert.equal(S.get(b.id).text,'Second draft');const disposable=S.create();disposable.text='discard only this';S.discard(disposable.id);assert.equal(S.get(disposable.id),undefined);assert.equal(S.get(b.id).text,'Second draft');
const r=S.get(a.id),p=S.submit(r,{text:r.text,files:r.files.map(f=>f.id)});assert.equal(r.text,'');r.text='Newer edits';S.acknowledge(r,p,{thread:'thread-authoritative',task_id:'task-accepted'});assert.equal(r.text,'Newer edits');assert.equal(r.thread,'thread-authoritative');assert.equal(r.pending.length,0);
const rejected=S.submit(r,{text:r.text,thread:r.thread,files:[]});r.text='Still newer';S.reject(r,rejected,'Validation refused');assert.equal(r.text,'Still newer');assert.equal(r.pending[0].body.text,'Newer edits');
const secondary=S.create(null,r.thread);secondary.localEntry=true;secondary.text='Recovered text';const acceptedSecondary=S.submit(secondary,{text:secondary.text,thread:r.thread,files:[]});secondary.text='Newer recovered edits';secondary.files=[{uid:randomUUID(),id:'file-new',name:'new.txt',state:'ready'}];S.acknowledge(secondary,acceptedSecondary,{thread:r.thread,task_id:'accepted-secondary'});assert.equal(secondary.localEntry,true);assert.equal(S.forThread(r.thread).id,r.id);S.changed(true);const secondaryId=secondary.id;S=fresh();S.open('appliance-a','alice');assert.equal(S.get(secondaryId).localEntry,true);assert.equal(S.get(secondaryId).text,'Newer recovered edits');S.discard(secondaryId);
// ACK with no concurrent edits must not retire an independent secondary identity.
const emptySecondary=S.create(null,r.thread);emptySecondary.localEntry=true;emptySecondary.text='Accepted secondary';const emptyAck=S.submit(emptySecondary,{text:emptySecondary.text,thread:r.thread,files:[]});S.acknowledge(emptySecondary,emptyAck,{thread:r.thread,task_id:'accepted-empty-secondary'});assert.equal(emptySecondary.localEntry,true);emptySecondary.text='Typed only after ACK';S.changed(true);const laterId=emptySecondary.id;S=fresh();S.open('appliance-a','alice');assert.equal(S.get(laterId).localEntry,true);assert.equal(S.get(laterId).text,'Typed only after ACK');assert.equal(S.forThread(r.thread).id,r.id);S.discard(laterId);
const exact=S.create();exact.text='  Preserve whitespace \n and caret  ';exact.selection=[4,17];const exactText=exact.text;const exactSubmission=S.submit(exact,{text:exact.text.trim(),files:[]});assert.equal(exactSubmission.draftText,exactText);assert.deepEqual([...exactSubmission.selection],[4,17]);S.reject(exact,exactSubmission,'Rejected');assert.equal(exact.text,exactText);assert.deepEqual([...exact.selection],[4,17]);S.discard(exact.id);
const erased=S.create();erased.text='Submitted before newer deletion';const erasedSend=S.submit(erased,{text:erased.text,files:[]});erased.editRevision=(erased.editRevision||0)+1;S.reject(erased,erasedSend,'Rejected after editing');assert.equal(erased.text,'');assert.equal(erased.pending.length,1);S.discard(erased.id);
const current=S.get(r.id);const unknown=S.submit(current,{text:current.text,thread:current.thread,files:[]});S.leave();S=fresh();S.open('appliance-a','alice');assert.equal(S.get(a.id).pending.find(x=>x.id===unknown.id).state,'unconfirmed');
const old=S.get(a.id);old.files=[{uid:randomUUID(),name:'interrupted.txt',state:'uploading'}];S.changed(true);S=fresh();S.open('appliance-a','alice');assert.equal(S.get(a.id).files[0].state,'failed');
S.originals.set('memory-only',{raw_upload_bytes:'NEVER-PERSIST-ME'});S.changed(true);assert.ok(![...storage.values()].join('').includes('NEVER-PERSIST-ME'));
S.leave();S.open('appliance-a','bob');assert.equal(S.all().length,0);S.open('appliance-b','alice');assert.equal(S.all().length,0);
S.open('appliance-a','alice');assert.equal(S.all().length,2);S.clear();S.leave();S=fresh();S.open('appliance-a','alice');assert.equal(S.all().length,0);
const memory=S.create();fail=true;memory.text='Retained despite disk failure';assert.equal(S.flush(),false);assert.equal(S.get(memory.id).text,'Retained despite disk failure');assert.equal(S.saved(),false);S.leave();S.open('appliance-a','alice');assert.equal(S.get(memory.id).text,'Retained despite disk failure');fail=false;S.flush();S.leave();
console.log('draft journal: distinct identities, restart, scoped sign-out, snapshots/newer edits, rejection/uncertainty, interrupted uploads and unavailable storage passed');
