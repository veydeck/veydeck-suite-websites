/** 浏览器验证主控语言通知、动态翻译、表单及页码保留和结构化错误。 */
const {chromium,expect}=require('../frontend/node_modules/@playwright/test');
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
(async()=>{const browser=await chromium.launch({headless:true});try{
 const now=Date.now(),certificate={id:'cert-1',domain:'demo.example',domains:['demo.example','alt.example'],issuerOrganization:'Veydeck',subject:'CN=demo.example',issuer:'CN=Veydeck',serialNumber:'123',fingerprintSha256:'AA:BB:CC',signatureAlgorithm:'ecdsa-with-SHA256',publicKey:'EC 256 bit',notBefore:now-1000,notAfter:now+365*86400000,chainLength:1};
 const sites=Array.from({length:25},(_,i)=>({id:`site-${i}`,revision:1,name:`Site ${i}`,directory:`site-${i}`,domains:[`site-${i}.example`],enabled:true,listenIp:'0.0.0.0',ipv6Enabled:false,port:8000+i,kind:'static',runDir:'.',spa:false,upstream:'',certificateId:null,advanced:''}));
 const state={sites,certificates:[certificate],hostDataDir:'/opt/veydeck/data/www/websites',runtimeConnected:true};
 const roots=[{...certificate,id:'root-1',name:'Veydeck',organization:'Veydeck',isDefault:true,createdAt:now,notAfter:253402300799000}];
 let rootDownloads=0;
 let performance={revision:0,serverNamesHashBucketSize:64,clientHeaderBufferSizeKb:1,clientMaxBodySizeMb:64,keepaliveTimeout:75,gzip:false,gzipMinLength:20,gzipCompLevel:1};
 const context=await browser.newContext({locale:'en-US',timezoneId:'Asia/Shanghai',viewport:{width:1400,height:950}});
 await context.route('http://suite.test/**',route=>{
  const uri=new URL(route.request().url()).pathname,method=route.request().method();
  if(uri==='/api/state')return route.fulfill({json:state});
  if(uri==='/api/certificate-authorities')return route.fulfill({json:roots});
  if(uri==='/api/settings/performance'){if(method==='PUT')performance={...route.request().postDataJSON(),revision:performance.revision+1};return route.fulfill({json:performance});}
  if(uri==='/api/sites'&&method==='POST')return route.fulfill({status:400,json:{code:'SITE_CREATE_FAILED_FILES_KEPT',error:'创建失败；文件已保留',params:{cause:{code:'DOMAIN_ALREADY_BOUND',error:'域名冲突',params:{domain:'pending.example',port:'8443'}},path:'/data/sites/pending.example/index'}}});
  if(uri==='/api/certificates'&&method==='POST')return route.fulfill({status:400,json:{code:'CERTIFICATE_KEY_MISMATCH',params:{},error:'证书和私钥不匹配'}});
  if(uri==='/api/backup')return route.fulfill({status:400,json:{code:'BACKUP_CONFIGURATION_BUSY',params:{},error:'发布过程中不能备份'}});
  if(uri.startsWith('/api/certificate-authorities/')&&uri.endsWith('/download')){rootDownloads++;return rootDownloads===1?route.fulfill({status:400,json:{code:'UNRECOGNIZED_ERROR',params:{},error:'未识别错误'}}):route.fulfill({status:401,json:{code:401,message:'会话已失效'}});}
  if(uri==='/api/logs')return route.fulfill({json:new URL(route.request().url()).searchParams.get('kind')==='error'?['原始 Nginx 诊断']: [JSON.stringify({time:'2026-10-22T12:00:00Z',remote:'127.0.0.1',method:'GET',uri:'/original-path',status:200,host:'demo.example'})]});
  const file=path.join(__dirname,'../frontend/dist',uri==='/'?'index.html':uri);
  return route.fulfill({body:fs.readFileSync(file),contentType:file.endsWith('.js')?'application/javascript':file.endsWith('.css')?'text/css':'text/html'});
 });
 // 独立模式使用 SDK 的浏览器语言快照，不能总是从中文开始。
 const standalone=await context.newPage();await standalone.goto('http://suite.test/');
 await expect(standalone.locator('html')).toHaveAttribute('lang','en-US');await expect(standalone).toHaveTitle('Website Manager');
 await expect(standalone.locator('[data-ui=workspace-menu]').getByRole('button',{name:'Settings',exact:true})).toHaveCount(0);
 await expect(standalone.locator('[data-ui=workspace-menu]').getByRole('button',{name:'Website list',exact:true})).toBeVisible();await standalone.close();
 const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.route('http://host.test/',route=>route.fulfill({contentType:'text/html',body:`<iframe src="http://suite.test/" style="position:fixed;inset:0;width:100%;height:100%;border:0"></iframe><script>
 window.currentLocale='zh-CN';window.messages=[];
 window.sendLocale=()=>document.querySelector('iframe').contentWindow.postMessage({protocolVersion:1,source:'veydeck-host',type:'host:locale:update',payload:{locale:currentLocale}},'http://suite.test');
 addEventListener('message',e=>{messages.push(e.data);if(e.data.type==='suite:lifecycle:ready')sendLocale();});
 </script>`}));
 await page.goto('http://host.test/');const ui=page.frameLocator('iframe'),menu=ui.locator('[data-ui=workspace-menu]');
 async function language(value){await page.evaluate(locale=>{window.currentLocale=locale;window.sendLocale();},value);await expect(ui.locator('html')).toHaveAttribute('lang',value);}
 async function go(name){await menu.getByRole('button',{name,exact:true}).click();}
 async function noUntranslatedText(){const text=await ui.locator('body').innerText();assert.ok(!/\p{Script=Han}/u.test(text),text);}
 await expect(menu.getByRole('button',{name:'网站列表',exact:true})).toBeVisible();await language('en-US');
 assert.deepEqual(await menu.getByRole('button').allTextContents(),['Website list','Certificate list','Self-signed certificates','Upload certificate','Performance','Logs','Backup']);
 await expect(ui.locator('html')).toHaveAttribute('lang','en-US');await noUntranslatedText();
 const table=ui.locator('[data-ui=sites-table]'),pager=table.locator('[data-ui=table-pagination]');
 await table.getByRole('button',{name:'Next page',exact:true}).click();await expect(pager).toHaveAttribute('data-page','2');
 await language('zh-CN');await expect(pager).toHaveAttribute('data-page','2');await language('en-US');await expect(pager).toHaveAttribute('data-page','2');
 await ui.getByRole('button',{name:'New website',exact:true}).click();
 await ui.getByLabel('Name',{exact:true}).fill('Pending site');await ui.locator('[data-ui=site-domains]').fill('pending.example');await ui.getByLabel('Port',{exact:true}).fill('8443');
 await language('zh-CN');await expect(ui.getByLabel('名称',{exact:true})).toHaveValue('Pending site');await expect(ui.locator('[data-ui=site-domains]')).toHaveValue('pending.example');
 await language('en-US');await ui.locator('[data-ui=save-site]').click();
 await expect(ui.locator('[data-ui=site-editor]')).toContainText('The domain pending.example is already bound on port 8443.');
 await expect(ui.locator('[data-ui=site-editor]')).toContainText('/data/sites/pending.example/index');await noUntranslatedText();
 await language('zh-CN');await expect(ui.locator('[data-ui=site-editor]')).toContainText('端口 8443 上的域名 pending.example 已绑定网站');
 await language('en-US');await ui.getByRole('button',{name:'Close',exact:true}).last().click();
 await go('Certificate list');await expect(ui.locator('[data-ui=certificates-table]')).toContainText('Valid');await noUntranslatedText();
 await ui.locator('[data-ui=certificates-table]').getByRole('button',{name:'Details',exact:true}).click();
 await expect(ui.locator('[data-ui=certificate-detail]')).toContainText('SHA-256 fingerprint');await noUntranslatedText();await ui.getByRole('button',{name:'Close',exact:true}).last().click();
 await go('Self-signed certificates');await noUntranslatedText();
 await ui.locator('[data-ui=authorities-table]').getByRole('button',{name:'Download',exact:true}).click();await expect(ui.locator('[data-page=certificate-authorities]')).toContainText('Request failed (HTTP 400).');
 await ui.locator('[data-ui=authorities-table]').getByRole('button',{name:'Download',exact:true}).click();await expect(ui.locator('[data-page=certificate-authorities]')).toContainText('The session has expired. Reopen the suite.');
 await ui.getByRole('button',{name:'Create root certificate',exact:true}).click();await ui.getByLabel('Root certificate name',{exact:true}).fill('Pending root');
 await language('zh-CN');await expect(ui.getByLabel('根证书名称',{exact:true})).toHaveValue('Pending root');await language('en-US');await noUntranslatedText();await ui.getByRole('button',{name:'Cancel',exact:true}).last().click();
 await ui.locator('[data-ui=authorities-table]').getByRole('button',{name:'Issue certificate',exact:true}).click();await noUntranslatedText();await ui.getByRole('button',{name:'Cancel',exact:true}).last().click();
 await go('Upload certificate');await noUntranslatedText();await ui.getByLabel('Certificate PEM',{exact:true}).fill('pending certificate');await ui.getByLabel('Private key PEM',{exact:true}).fill('pending key');
 await ui.locator('[data-ui=certificate-upload]').getByRole('button',{name:'Upload certificate',exact:true}).click();await expect(ui.locator('[data-ui=certificate-upload]')).toContainText('The certificate and private key do not match.');
 await language('zh-CN');await expect(ui.getByLabel('私钥 PEM',{exact:true})).toHaveValue('pending key');await expect(ui.locator('[data-ui=certificate-upload]')).toContainText('证书和私钥不匹配');await language('en-US');
 await go('Performance');const panel=ui.locator('[data-ui=performance-settings]'),level=panel.locator('[data-ui=performance-gzipCompLevel] input, input[data-ui=performance-gzipCompLevel]');
 await level.fill('10');await panel.getByRole('button',{name:'Save and reload',exact:true}).click();await expect(panel).toContainText('higher levels use more CPU');await noUntranslatedText();
 await language('zh-CN');await expect(panel).toContainText('压缩级别');await expect(level).toHaveValue('10');await language('en-US');await level.fill('3');await panel.getByRole('button',{name:'Save and reload',exact:true}).click();await expect(panel).toContainText('Saved and reloaded');await language('zh-CN');await expect(panel).toContainText('已保存并重载');await language('en-US');
 await go('Logs');await expect(ui.locator('[data-ui=log-text]')).toContainText('原始 Nginx 诊断');
 await ui.locator('[data-ui=engine-log-kind]').click();await ui.getByRole('option',{name:'Unmatched website access log',exact:true}).click();await expect(ui.locator('[data-ui=log-table]')).toContainText('GET /original-path');await noUntranslatedText();
 await go('Backup');await noUntranslatedText();await ui.getByRole('button',{name:'Download backup',exact:true}).click();await expect(ui.locator('main')).toContainText('Wait for the configuration change to finish before backing up.');await language('zh-CN');await expect(ui.locator('main')).toContainText('发布过程中不能备份');await language('en-US');
 // 英文按钮比中文长，确认表格中的操作没有被单元格裁剪。
 for(const [label,id]of [['Website list','sites-table'],['Certificate list','certificates-table'],['Self-signed certificates','authorities-table']]){
  await go(label);const list=ui.locator(`[data-ui=${id}]`);await expect(list.locator('[data-ui=table-actions]').first()).toBeVisible();
  const overflows=await list.locator('[data-ui=table-actions]').evaluateAll(groups=>groups.map(el=>el.scrollWidth-el.clientWidth));assert.ok(overflows.every(v=>v<=1),`${id}: ${overflows}`);
  await page.screenshot({path:`/tmp/veydeck-i18n-${id}.png`});
 }
 await page.setViewportSize({width:480,height:760});assert.equal(await ui.locator('html').evaluate(el=>el.scrollWidth<=innerWidth),true);
 await language('zh-CN');await expect(menu.getByRole('button',{name:'自签证书',exact:true})).toHaveAttribute('aria-current','page');await language('en-US');
 assert.deepEqual(errors,[]);console.log('PASS: SDK 浏览器/主控语言、全页英文、实时切换、输入/页码/错误/成功提示保留、日期、原始日志、英文布局与窄窗口');
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exit(1)});
