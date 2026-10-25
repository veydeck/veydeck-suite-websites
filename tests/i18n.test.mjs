/** 语言资源完整性、占位符及业务错误码的契约检查。 */
import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url),ts=require('../frontend/node_modules/typescript');
const {parse}=require('../frontend/node_modules/vue/compiler-sfc');
const messages=JSON.parse(fs.readFileSync(new URL('../frontend/src/locales/messages.json',import.meta.url)));
const errors=JSON.parse(fs.readFileSync(new URL('../frontend/src/locales/errors.json',import.meta.url)));
const placeholders=text=>[...text.matchAll(/\{([a-zA-Z]\w*)\}/g)].map(m=>m[1]).sort();
test('中英文资源完整且插值参数一致',()=>{
 for(const [zh,en] of Object.entries(messages)){assert.ok(en.trim(),zh);assert.deepEqual(placeholders(zh),placeholders(en),zh);assert.ok(!/\p{Script=Han}/u.test(en),zh);}
 for(const [code,entry]of Object.entries(errors)){assert.ok(entry['zh-CN']&&entry['en-US'],code);assert.deepEqual(placeholders(entry['zh-CN']),placeholders(entry['en-US']),code);}
});
test('所有公开业务错误码均有中英文资源',()=>{
 for(const file of fs.readdirSync(new URL('../backend/src/',import.meta.url)).filter(f=>f.endsWith('.rs'))){
  const source=fs.readFileSync(new URL('../backend/src/'+file,import.meta.url),'utf8').split('#[cfg(test)]')[0];
  for(const match of source.matchAll(/(?:Error::bad|Error::with_params|require)\(\s*"([A-Z_]+)"/g))assert.ok(errors[match[1]],`${file}: ${match[1]}`);
  for(const match of source.matchAll(/"([A-Z_]+)",\s*"[^"\n]*\p{Script=Han}[^"\n]*"/gu))assert.ok(errors[match[1]],`${file}: ${match[1]}`);
 }
});
test('界面无硬编码中文文本和属性，脚本文案均纳入词典',()=>{
 for(const file of fs.readdirSync(new URL('../frontend/src/',import.meta.url)).filter(f=>/\.(vue|ts)$/.test(f))){
  const source=fs.readFileSync(new URL('../frontend/src/'+file,import.meta.url),'utf8');let script=source;
  if(file.endsWith('.vue')){
   const descriptor=parse(source).descriptor;script=descriptor.scriptSetup?.content??'';
   const visit=node=>{if(node.type===2)assert.ok(!/\p{Script=Han}/u.test(node.content),`${file}: ${node.content}`);for(const prop of node.props??[])if(prop.type===6&&prop.value)assert.ok(!/\p{Script=Han}/u.test(prop.value.content),`${file}: ${prop.loc.source}`);for(const child of node.children??[])visit(child);};
   visit(descriptor.template.ast);
  }
  const ast=ts.createSourceFile(file+'.ts',script,ts.ScriptTarget.Latest,true);
  const visit=node=>{if(ts.isStringLiteral(node)&&/\p{Script=Han}/u.test(node.text))assert.ok(Object.hasOwn(messages,node.text),`${file}: ${node.text}`);ts.forEachChild(node,visit);};visit(ast);
 }
});
