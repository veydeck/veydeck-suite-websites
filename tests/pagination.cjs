/** 使用本地构建产物验证固定高度列表、尺寸分页及删除后的页码修正。 */
const {chromium,expect}=require('../frontend/node_modules/@playwright/test');
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
(async()=>{const browser=await chromium.launch({headless:true});try{
 const now=Date.now();
 const certificate=(i)=>({id:`cert-${i}`,domain:`cert-${i}.example`,domains:[`cert-${i}.example`,'very-long-domain.'.repeat(30)+'example'],issuerOrganization:'Veydeck',notBefore:now-1000,notAfter:now+365*86400000,subject:'CN=test',issuer:'CN=Veydeck',serialNumber:String(i),fingerprintSha256:'AA:BB:CC',signatureAlgorithm:'ecdsa-with-SHA256',publicKey:'EC 256 bit',chainLength:1});
 const state={sites:Array.from({length:121},(_,i)=>({id:`site-${i}`,revision:1,name:`网站 ${i}`,directory:`site-${i}`,domains:[`site-${i}.example`],enabled:true,listenIp:'0.0.0.0',ipv6Enabled:false,port:8000+i,kind:'static',runDir:'.',spa:false,upstream:'',certificateId:null,advanced:''})),certificates:Array.from({length:121},(_,i)=>certificate(i)),hostDataDir:'/opt/veydeck/data/www/websites',runtimeConnected:true};
 const roots=Array.from({length:121},(_,i)=>({...certificate(i),name:`根证书 ${i}`,organization:'Veydeck',isDefault:i===0,createdAt:now}));
 const context=await browser.newContext({locale:'zh-CN',viewport:{width:1280,height:535}});
 const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.route('http://suite.test/**',route=>{
  const uri=new URL(route.request().url()).pathname;
  if(uri==='/api/state')return route.fulfill({json:state});
  if(uri==='/api/certificate-authorities')return route.fulfill({json:roots});
  if(uri.startsWith('/api/certificates/')&&route.request().method()==='DELETE'){state.certificates=state.certificates.filter(c=>c.id!==uri.split('/').at(-1));return route.fulfill({status:204});}
  const file=path.join(__dirname,'../frontend/dist',uri==='/'?'index.html':uri);
  return route.fulfill({body:fs.readFileSync(file),contentType:file.endsWith('.js')?'application/javascript':file.endsWith('.css')?'text/css':'text/html'});
 });
 await page.goto('http://suite.test/');
 const menu=page.locator('[data-ui=workspace-menu]');
 async function checkFits(table){
  await expect(async()=>{
   const layout=await table.evaluate(el=>{const main=el.closest('main'),content=el.closest('[data-slot=content]'),wrapper=el.querySelector('.vl-table-wrapper'),footer=el.querySelector('[data-ui=table-pagination]');return {contentOverflow:content.scrollHeight-content.clientHeight,tableOverflow:wrapper.scrollHeight-wrapper.clientHeight,mainOverflow:main.scrollHeight-main.clientHeight,bottom:footer.getBoundingClientRect().bottom,viewport:innerHeight};});
   assert.ok(layout.contentOverflow<=1,JSON.stringify(layout));assert.ok(layout.tableOverflow<=1,JSON.stringify(layout));assert.ok(layout.mainOverflow<=1,JSON.stringify(layout));assert.ok(layout.bottom<=layout.viewport,JSON.stringify(layout));
  }).toPass();
 }
 for(const [label,id] of [['网站列表','sites-table'],['证书列表','certificates-table'],['自签证书','authorities-table']]){
  await menu.getByRole('button',{name:label,exact:true}).click();
  const table=page.locator(`[data-ui=${id}]`),pager=table.locator('[data-ui=table-pagination]');
  await expect(pager).toHaveAttribute('data-total-pages',/^[2-9][0-9]*$|^1[0-9]+$/);
  await expect(async()=>assert.ok(Number(await pager.getAttribute('data-page-size'))>1)).toPass();
  const size=Number(await pager.getAttribute('data-page-size'));
  await expect(table.locator('tbody tr')).toHaveCount(size);
  await checkFits(table);
  const first=await table.locator('tbody tr').first().textContent();
  await table.getByRole('button',{name:'下一页',exact:true}).click();await expect(pager).toHaveAttribute('data-page','2');
  assert.notEqual(await table.locator('tbody tr').first().textContent(),first);
  const last=Number(await pager.getAttribute('data-total-pages'));
  await table.getByRole('button',{name:`Page ${last}`,exact:true}).click();await expect(pager).toHaveAttribute('data-page',String(last));
  await expect(table.getByRole('button',{name:'下一页',exact:true})).toBeDisabled();
  await checkFits(table);
  await page.setViewportSize({width:1280,height:900});await expect(async()=>assert.ok(Number(await pager.getAttribute('data-page-size'))>size)).toPass();
  await checkFits(table);
  await page.setViewportSize({width:480,height:535});await checkFits(table);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
  await page.screenshot({path:`/tmp/veydeck-pagination-${id}.png`});
  await page.setViewportSize({width:1280,height:535});
 }
 await menu.getByRole('button',{name:'证书列表',exact:true}).click();
 const table=page.locator('[data-ui=certificates-table]'),pager=table.locator('[data-ui=table-pagination]');
 await expect(async()=>assert.ok(Number(await pager.getAttribute('data-page-size'))>1)).toPass();
 const size=Number(await pager.getAttribute('data-page-size'));
 state.certificates=state.certificates.slice(0,size+1);
 await page.getByRole('button',{name:'刷新',exact:true}).click();await expect(pager).toHaveAttribute('data-total-pages','2');
 await table.getByRole('button',{name:'下一页',exact:true}).click();await expect(table.locator('tbody tr')).toHaveCount(1);
 await table.getByRole('button',{name:'删除',exact:true}).click();await page.getByRole('button',{name:'确认删除',exact:true}).click();
 await expect(pager).toHaveAttribute('data-page','1');await expect(pager).toHaveAttribute('data-total-pages','1');await checkFits(table);
 state.certificates=[];await page.getByRole('button',{name:'刷新',exact:true}).click();await expect(pager).toContainText('共 0 条');await checkFits(table);
 await page.evaluate(()=>document.documentElement.dataset.theme='dark');await page.screenshot({path:'/tmp/veydeck-pagination-empty-dark.png'});
 assert.deepEqual(errors,[]);
 console.log('PASS: 121 条数据的三表翻页、固定底部、无纵向溢出、窗口自适应、长域名、末页删除修正、空状态');
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exit(1)});
