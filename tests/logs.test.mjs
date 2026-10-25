/** 验证日志转换保留内容。 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { logEntries } from '../frontend/src/logs.ts';
test('访问日志显示来源、请求和状态',()=>{
 const [entry]=logEntries([JSON.stringify({time:'2026-10-22T12:00:00Z',remote:'2001:db8::1',method:'GET',uri:'/中文',status:404,host:'example.com'})]);
 assert.equal(entry.source,'2001:db8::1');assert.equal(entry.request,'GET /中文');assert.equal(entry.status,'404');assert.equal(entry.detail,'example.com');assert.notEqual(entry.time,'—');
});
test('无法解析及未知结构不丢弃原始文本',()=>{
 const lines=['不完整的日志 {',JSON.stringify({unexpected:'未知字段'}),'"文本日志"'];
 const entries=logEntries(lines);
 assert.equal(entries[0].detail,lines[0]);assert.equal(entries[1].detail,lines[1]);assert.equal(entries[2].detail,'文本日志');
});
test('日志时间按界面语言显示，原始请求和诊断不翻译',()=>{
 const lines=[JSON.stringify({time:'2026-10-22T12:00:00Z',remote:'127.0.0.1',method:'GET',uri:'/中文',status:200,host:'example.com'}),'原始诊断'];
 const zh=logEntries(lines,'zh-CN'),en=logEntries(lines,'en-US');
 assert.notEqual(zh[0].time,en[0].time);assert.equal(en[0].request,'GET /中文');assert.equal(en[1].detail,'原始诊断');
});
