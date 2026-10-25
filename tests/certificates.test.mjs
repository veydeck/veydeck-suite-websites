/** 验证有效期边界与语言、时区转换，避免依赖测试机时区。 */
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {certificateStatus,formatCertificateTime} from '../frontend/src/certificates.ts';
test('有效期状态包含未来、有效、临期和过期边界',()=>{
 const day=86400000, cert={notBefore:1000,notAfter:1000+60*day};
 assert.equal(certificateStatus(cert,999).label,'尚未生效');
 assert.equal(certificateStatus(cert,1000).label,'有效');
 assert.equal(certificateStatus(cert,1000+30*day).label,'即将过期');
 assert.equal(certificateStatus(cert,cert.notAfter).label,'已过期');
});
test('同一 UTC 时间按中文和英文及时区显示，不再输出原始 GMT 字符串',()=>{
 const timestamp=Date.UTC(2027,10,23,14,41,1);
 const shanghai=formatCertificateTime(timestamp,'zh-CN','Asia/Shanghai');
 const losAngeles=formatCertificateTime(timestamp,'en-US','America/Los_Angeles');
 assert.match(shanghai,/2027/);assert.match(shanghai,/22:41:01/);
 assert.match(losAngeles,/06:41:01/);assert.notEqual(shanghai,losAngeles);
 assert.equal(formatCertificateTime(NaN),'—');
});

test('标准未定义到期日固定显示 UTC，不因时区换算变成 10000 年',()=>{
 for(const timezone of ['Asia/Shanghai','America/Los_Angeles'])assert.equal(formatCertificateTime(253402300799000,'zh-CN',timezone),'9999-12-31 23:59:59 UTC');
});
