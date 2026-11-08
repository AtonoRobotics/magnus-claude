import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
const root=join(dirname(fileURLToPath(import.meta.url)),'../..');
const {chromium}=createRequire(import.meta.url)(join(root,'browser/node_modules/playwright-core'));
const browser=await chromium.launch({executablePath:process.env.MAGNUS_CHROMIUM,args:['--no-sandbox','--disable-dev-shm-usage']});
const context=await browser.newContext();const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
const base=process.env.MAGNUS_URL;
const signIn=async person=>{await page.getByLabel('Username').fill(person);await page.getByLabel('Password').fill(process.env.MAGNUS_PASSWORD);await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.locator('#msg').waitFor();};
const until=async(fn,arg)=>{const end=Date.now()+30000;while(Date.now()<end){if(await page.evaluate(fn,arg))return;await page.waitForTimeout(40);}throw Error('UI condition timed out');};
const draft=()=>page.evaluate(()=>App.draftRecord.id);
const newDraft=async()=>{const old=await draft();await page.getByRole('link',{name:'New conversation',exact:true}).click();await until(old=>App.draftRecord?.id!==old,old);return draft();};
const openDraft=async id=>{await page.locator(`nav.side a[href="#/draft/${id}"]`).click();await until(id=>App.draftRecord?.id===id,id);};
try {
 await page.goto(base);await signIn('alice');
 const first=await draft();await page.locator('#msg').fill('First retained draft');
 let releaseUpload,uploaded;const uploadHeld=new Promise(resolve=>releaseUpload=resolve);
 await page.route('**/v1/files',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();uploaded=await response.json();await uploadHeld;await route.fulfill({response});});
 await page.locator('#say input[data-attach]').setInputFiles({name:'notes.txt',mimeType:'text/plain',buffer:Buffer.from('shared file bytes')});
 await page.locator('.attachment-chip').getByText('Uploading…').waitFor();
 await page.getByRole('link',{name:'Settings',exact:true}).click();releaseUpload();
 await until(id=>MessageDrafts.get(id)?.files[0]?.state==='ready',first);
 await openDraft(first);assert.equal(await page.locator('#msg').inputValue(),'First retained draft');await page.locator('.attachment-chip').getByText('Ready',{exact:true}).waitFor();
 await page.unroute('**/v1/files');
 const second=await newDraft();assert.notEqual(first,second);
 await page.locator('#msg').fill('Second retained draft');await page.reload();await page.locator('#msg').waitFor();
 assert.equal(await page.locator('#msg').inputValue(),'Second retained draft');await openDraft(first);assert.equal(await page.locator('#msg').inputValue(),'First retained draft');
 assert.equal(await page.locator('.attachment-chip').count(),1);
 await page.getByRole('button',{name:'Remove notes.txt from message'}).click();assert.equal(await page.locator('.attachment-chip').count(),0);
 assert.equal((await context.request.get(`${base}/v1/files/${uploaded.id}/info`)).status(),200,'removing attachment must not delete shared file');
 // Opening and Back/close preserve the selected draft and use deliberate focus.
 await page.getByRole('link',{name:'Files',exact:true}).click();await page.locator('#detail [data-panel=files]').waitFor();
 await page.locator(`#detail [data-open="file:${uploaded.id}"]`).click();await page.locator('#detail [data-panel=file]').waitFor();
 await until(()=>document.activeElement?.tagName==='H2' && !!document.activeElement.closest('#detail'));
 assert.equal(await page.locator('#msg').inputValue(),'First retained draft');
 await page.goBack();await page.locator('#detail [data-panel=files]').waitFor();
 await page.getByRole('button',{name:'Close work',exact:true}).click();
 assert.ok(await page.evaluate(()=>document.activeElement===document.querySelector('#conv-head h1')));
 // A removed pending upload may finish, but must never reattach itself.
 let releaseLate,lateFile;const lateHeld=new Promise(resolve=>releaseLate=resolve);
 await page.route('**/v1/files',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();lateFile=await response.json();await lateHeld;await route.fulfill({response});});
 await page.locator('#say input[data-attach]').setInputFiles({name:'late.txt',mimeType:'text/plain',buffer:Buffer.from('late completion')});
 await page.locator('.attachment-chip').getByText('Uploading…').waitFor();await page.getByRole('button',{name:'Remove late.txt from message'}).click();const lateFinished=page.waitForEvent('requestfinished',{predicate:r=>r.method()==='POST' && new URL(r.url()).pathname==='/v1/files'});releaseLate();await lateFinished;
 await until(()=>MessageDrafts.all().every(r=>r.files.every(f=>f.name!=='late.txt')));await page.unroute('**/v1/files');

 // Restart during an unfinished upload retains metadata, never raw bytes.
 const interrupted=await newDraft();await page.locator('#msg').fill('Interrupted upload draft');
 let endInterrupted;const interruptedHeld=new Promise(resolve=>endInterrupted=resolve);
 await page.route('**/v1/files',async route=>{if(route.request().method()!=='POST')return route.continue();await interruptedHeld;try{await route.abort('failed');}catch{/* reload already aborted it */}});
 await page.locator('#say input[data-attach]').setInputFiles({name:'interrupted.txt',mimeType:'text/plain',buffer:Buffer.from('PRIVATE-UPLOAD-BYTES')});
 await page.locator('.attachment-chip').getByText('Uploading…').waitFor();await page.reload();
 await page.locator('.attachment-chip').getByText('Failed',{exact:true}).waitFor();assert.equal(await page.locator('#msg').inputValue(),'Interrupted upload draft');
 assert.equal(await page.evaluate(()=>JSON.stringify(localStorage).includes('PRIVATE-UPLOAD-BYTES')),false);
 endInterrupted();await page.unroute('**/v1/files');await openDraft(first);
 // IME Enter must leave the draft and perform no send.
 const before=(await(await context.request.get(`${base}/v1/chat`)).json()).threads.length;
 await page.locator('#msg').dispatchEvent('keydown',{key:'Enter',code:'Enter',isComposing:true});
 assert.equal((await(await context.request.get(`${base}/v1/chat`)).json()).threads.length,before);
 assert.equal(await page.locator('#msg').inputValue(),'First retained draft');
 // The server accepts first send, while the user continues typing before ACK.
 let releaseSend,accepted;const sendHeld=new Promise(resolve=>releaseSend=resolve);
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();accepted=await response.json();await sendHeld;await route.fulfill({response});});
 await page.locator('#send').click();await until(()=>document.querySelector('#msg').value==='');
 await page.locator('#msg').fill('Newer text while sending');releaseSend();
 await page.waitForURL(/#\/chat\//);await until(()=>document.querySelector('#msg')?.value==='Newer text while sending');
 assert.equal(await page.locator(`nav.side a[href="#/draft/${first}"]`).count(),0,'ACK replaces local new-conversation entry');
 await page.reload();await until(()=>document.querySelector('#msg')?.value==='Newer text while sending');
 await page.unroute('**/v1/chat');
 // An actual accepted message whose HTTP response is lost stays unconfirmed.
 const uncertain=await newDraft();await page.locator('#msg').fill('Response will be lost');
 let lostThread,postCount=0;
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();postCount++;const response=await route.fetch();lostThread=(await response.json()).thread;await route.abort('failed');});
 await page.locator('#send').click();await page.locator('#submitted').getByText('Send outcome not confirmed',{exact:true}).waitFor();
 await page.locator('#msg').fill('Newer edits after lost response');await page.reload();
 await page.locator('#submitted').getByText('Send outcome not confirmed',{exact:true}).waitFor();assert.equal(await page.locator('#msg').inputValue(),'Newer edits after lost response');assert.equal(postCount,1);
 const acceptedLost=(await(await context.request.get(`${base}/v1/chat/${lostThread}`)).json()).entries.filter(e=>e.kind==='message');assert.equal(acceptedLost.length,1);
 await page.unroute('**/v1/chat');
 // Definite server validation rejection keeps submitted and newer text separate.
 const rejectedText='  Rejected submitted text \n with original whitespace  ';
 const rejectedId=await newDraft();await page.locator('#msg').fill(rejectedText);await page.locator('#msg').evaluate(box=>box.setSelectionRange(5,19));
 const validModel=await page.locator('#say [data-model]').inputValue();
 await page.locator('#say [data-model]').evaluate(select=>{const option=new Option('Removed model',JSON.stringify(['missing-adapter','removed-model']));select.append(option);select.value=option.value;});
 let releaseRejected;const rejectedHeld=new Promise(resolve=>releaseRejected=resolve);
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();assert.equal(response.status(),400);await rejectedHeld;await route.fulfill({response});});
 await page.locator('#send').click();await until(()=>document.querySelector('#msg').value==='');await page.locator('#msg').fill('Newer edits after rejection');releaseRejected();
 await page.locator('#submitted').getByText('Message was not accepted',{exact:true}).waitFor();assert.equal(await page.locator('#msg').inputValue(),'Newer edits after rejection');
 await page.unroute('**/v1/chat');await page.getByRole('button',{name:'Edit rejected message'}).click();await until(text=>document.querySelector('#msg')?.value===text,rejectedText);assert.deepEqual(await page.locator('#msg').evaluate(box=>[box.selectionStart,box.selectionEnd]),[5,19]);
 assert.equal(await page.evaluate(id=>MessageDrafts.get(id).text,rejectedId),'Newer edits after rejection');await page.locator('#say [data-model]').selectOption(validModel);
 { // A rejection from an unmounted request recovers into the newly mounted composer.
 await page.goto(`${base}/#/chat/${accepted.thread}`);await page.locator('#msg').fill('Late rejection after remount');
 const invalid=async()=>page.locator('#say [data-model]').evaluate(select=>{const option=new Option('Removed model',JSON.stringify(['missing-adapter','removed-model']));select.append(option);select.value=option.value;});
 await invalid();let releaseLate;const lateHeld=new Promise(resolve=>releaseLate=resolve);
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();assert.equal(response.status(),400);await lateHeld;await route.fulfill({response});});
 await page.locator('#send').click();await until(()=>document.querySelector('#msg').value==='');
 await page.getByRole('link',{name:'Settings',exact:true}).click();await page.locator(`nav.side a[href="#/chat/${accepted.thread}"]`).click();await page.locator('#msg').waitFor();releaseLate();
 await until(()=>document.querySelector('#msg')?.value==='Late rejection after remount');await page.unroute('**/v1/chat');
 await page.getByRole('link',{name:'Settings',exact:true}).click();await page.locator(`nav.side a[href="#/chat/${accepted.thread}"]`).click();assert.equal(await page.locator('#msg').inputValue(),'Late rejection after remount');
 // A recovered secondary thread draft stays discoverable when acknowledgement leaves newer edits.
 await invalid();let releaseSecondary;const secondaryHeld=new Promise(resolve=>releaseSecondary=resolve);
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();assert.equal(response.status(),400);await secondaryHeld;await route.fulfill({response});});
 await page.locator('#send').click();await until(()=>document.querySelector('#msg').value==='');await page.locator('#msg').fill('Original independent thread draft');releaseSecondary();await page.getByRole('button',{name:'Edit rejected message'}).waitFor();await page.unroute('**/v1/chat');
 await page.getByRole('button',{name:'Edit rejected message'}).click();await until(()=>location.hash.startsWith('#/draft/') && App.draftRecord?.localEntry && document.querySelector('#msg')?.value==='Late rejection after remount');const recoveredId=await page.evaluate(()=>App.draftRecord.id);await page.locator('#say [data-model]').selectOption(validModel);
 let releaseRecovered;const recoveredHeld=new Promise(resolve=>releaseRecovered=resolve);
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();assert.equal(response.status(),201);await recoveredHeld;await route.fulfill({response});});
 await page.locator('#send').click();await until(()=>document.querySelector('#msg').value==='');releaseRecovered();await until(()=>!App.draftRecord.pending.length);assert.equal(await page.evaluate(()=>App.draftRecord.localEntry),true);await page.locator('#msg').fill('Additional recovered draft edits');
 await page.locator('#say [data-attach]').setInputFiles({name:'newer.txt',mimeType:'text/plain',buffer:Buffer.from('newer attachment')});await page.locator('#pending').getByText('Ready',{exact:true}).waitFor();await page.unroute('**/v1/chat');
 assert.equal(await page.evaluate(()=>App.draftRecord.localEntry),true);await page.reload();await until(()=>document.querySelector('#msg')?.value==='Additional recovered draft edits');await page.locator('#pending').getByText('newer.txt',{exact:true}).waitFor();
 await page.getByRole('link',{name:'Settings',exact:true}).click();await page.locator(`nav.side a[href="#/draft/${recoveredId}"]`).click();await until(()=>document.querySelector('#msg')?.value==='Additional recovered draft edits');assert.equal(await page.evaluate(thread=>MessageDrafts.forThread(thread).text,accepted.thread),'Original independent thread draft');
 }
 // Definite rejection restores the original raw input and caret when no newer edits exist.
 const exactId=await newDraft(),exactText='  Exact rejected message \n trailing spaces  ';await page.locator('#msg').fill(exactText);await page.locator('#msg').evaluate(box=>box.setSelectionRange(3,16));
 await page.locator('#say [data-model]').evaluate(select=>{const option=new Option('Removed model',JSON.stringify(['missing-adapter','removed-model']));select.append(option);select.value=option.value;});
 await page.locator('#send').click();await until(text=>document.querySelector('#msg')?.value===text,exactText);assert.deepEqual(await page.locator('#msg').evaluate(box=>[box.selectionStart,box.selectionEnd]),[3,16]);assert.equal(await page.evaluate(id=>MessageDrafts.get(id).pending.length,exactId),0);
 await page.getByRole('link',{name:'Settings',exact:true}).click();assert.deepEqual(await page.evaluate(id=>MessageDrafts.get(id).selection,exactId),[3,16],'journal selection on leaving');await openDraft(exactId);assert.equal(await page.locator('#msg').inputValue(),exactText);assert.deepEqual(await page.locator('#msg').evaluate(box=>({selection:[box.selectionStart,box.selectionEnd],stored:App.draftRecord.selection,connected:box.form.isConnected,hash:location.hash})),{selection:[3,16],stored:[3,16],connected:true,hash:`#/draft/${exactId}`});
 // Typing and then erasing is still a newer edit, not permission to restore over it.
 await newDraft();await page.locator('#msg').fill('Do not overwrite a newer intentional deletion');await page.locator('#say [data-model]').evaluate(select=>{const option=new Option('Removed model',JSON.stringify(['missing-adapter','removed-model']));select.append(option);select.value=option.value;});
 let releaseErased;const erasedHeld=new Promise(resolve=>releaseErased=resolve);await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();assert.equal(response.status(),400);await erasedHeld;await route.fulfill({response});});
 await page.locator('#send').click();await until(()=>document.querySelector('#msg').value==='');await page.locator('#msg').fill('Temporary newer input');await page.locator('#msg').fill('');releaseErased();await page.getByRole('button',{name:'Edit rejected message'}).waitFor();assert.equal(await page.locator('#msg').inputValue(),'');await page.unroute('**/v1/chat');
 // Controlled observer delivery reproduces a deliberate scroll during resize restoration.
 await page.evaluate(()=>{
   const NativeObserver=window.ResizeObserver;let resize;
   window.ResizeObserver=class{constructor(fn){resize=fn;}observe(){}disconnect(){}};
   const root=document.createElement('div');root.style.cssText='position:fixed;left:0;top:0;width:300px;height:100px;overflow:auto';root.innerHTML=Array.from({length:30},(_,n)=>`<p data-key="race-${n}" style="height:40px">Readable row ${n}</p>`).join('');document.body.append(root);
   let unwatch;
   try{unwatch=ReadingAnchors.watch(root,{anchor:{follow:true}},false);resize();root.scrollTop=150;root.dispatchEvent(new Event('scroll'));resize();if(Math.abs(root.scrollTop-150)>1)throw Error('Resize restoration swallowed deliberate scrolling');}
   finally{unwatch?.();root.remove();window.ResizeObserver=NativeObserver;}
 });
 // Scroll anchoring uses an entry identity; new data never forces latest.
 for(let i=0;i<20;i++){const response=await page.evaluate(body=>Magnus.api('POST','/v1/chat',body),{thread:accepted.thread,text:`Anchor entry ${i}`});assert.equal(response.thread,accepted.thread);}
 await page.goto(`${base}/#/chat/${accepted.thread}`);const expectedMessages=await page.evaluate(async thread=>(await Magnus.api('GET',`/v1/chat/${thread}`)).entries.filter(e=>e.kind==='message').length,accepted.thread);await page.locator('.entry[data-kind=message]').nth(expectedMessages-1).waitFor();
 await page.locator('#thread').evaluate(el=>{el.scrollTop=120;});await page.waitForTimeout(350);
 const anchor=()=>page.locator('#thread').evaluate(el=>{const top=el.getBoundingClientRect().top;const n=[...el.querySelectorAll('[data-entry]')].find(n=>n.getBoundingClientRect().bottom>top);return{id:n.dataset.entry,offset:n.getBoundingClientRect().top-top,bottom:el.scrollHeight-el.scrollTop-el.clientHeight};});
 const previous=await anchor();assert.ok(previous.bottom>80);
 const response=await page.evaluate(body=>Magnus.api('POST','/v1/chat',body),{thread:accepted.thread,text:'Activity while reading above'});assert.equal(response.thread,accepted.thread);
 await page.locator('.entry[data-kind=message]').nth(expectedMessages).waitFor();const after=await anchor();assert.equal(after.id,previous.id);assert.ok(Math.abs(after.offset-previous.offset)<3);
 // A real uploaded image gains intrinsic size above the reader after hydration.
 const image=await page.evaluate(body=>Magnus.api('POST','/v1/files',body),{name:'anchor-pixel.png',media_type:'image/png',data_b64:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII='});
 await page.evaluate(async id=>{const first=document.querySelector('#thread [data-entry]'),img=document.createElement('img');img.alt='Delayed intrinsic-size anchor fixture';img.style.width='160px';img.dataset.proseImage=`/v1/files/${id}`;first.append(img);loadMarkdownImages(first);await new Promise((resolve,reject)=>{img.onload=resolve;img.onerror=reject;});await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));},image.id);
 const afterImage=await anchor();assert.equal(afterImage.id,previous.id);assert.ok(Math.abs(afterImage.offset-previous.offset)<3,'late intrinsic image size preserves the reader anchor');
 await page.evaluate(()=>{const stamp=document.querySelector('#thread .entry .small');if(stamp)stamp.textContent='Magnus · refreshed timestamp';});await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));const afterStamp=await anchor();assert.equal(afterStamp.id,previous.id);assert.ok(Math.abs(afterStamp.offset-previous.offset)<3,'timestamp-only changes do not follow latest');
 await page.getByRole('button',{name:'New activity · Jump to latest'}).click();assert.ok((await anchor()).bottom<=80);
 await page.evaluate(body=>Magnus.api('POST','/v1/chat',body),{thread:accepted.thread,text:'Follow new activity while already at bottom'});await page.locator('.entry[data-kind=message]').nth(expectedMessages+1).waitFor();await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));assert.ok((await anchor()).bottom<=80,'readers at bottom keep following actual new content');

 // Unavailable storage keeps the current draft through ordinary navigation.
 await page.evaluate(()=>{window.originalStorageSet=Storage.prototype.setItem;Storage.prototype.setItem=function(){throw new Error('test storage unavailable');};});
 await page.locator('#msg').fill('Unsaved text stays in memory');await page.getByText('Draft not saved on this device',{exact:true}).waitFor();
 await page.getByRole('link',{name:'Settings',exact:true}).click();
 await page.locator(`nav.side a[href="#/chat/${accepted.thread}"]`).click();await until(()=>document.querySelector('#msg')?.value==='Unsaved text stays in memory');
 await page.evaluate(()=>{Storage.prototype.setItem=window.originalStorageSet;delete window.originalStorageSet;MessageDrafts.flush();});
 // Explicit sign-out clears Alice's message journal; Bob sees none of it.
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await signIn('bob');
 assert.equal(await page.evaluate(()=>MessageDrafts.all().some(r=>r.text.includes('retained')||r.pending.some(p=>p.body.text==='Response will be lost'))),false);
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await signIn('alice');
 assert.equal(await page.evaluate(id=>!!MessageDrafts.get(id),uncertain),false);
 assert.deepEqual(errors,[]);console.log('browser: distinct drafts/restart, uploads across navigation, nondeleting removal, IME, newer edits after ACK/lost response, anchors, work focus/history, storage failure and scoped sign-out passed');
} finally {await browser.close();}
