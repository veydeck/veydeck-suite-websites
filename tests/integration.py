#!/usr/bin/env python3
"""真实 Nginx 容器集成验证；用受限 UDS 桩模拟 Agent 的固定操作传输。"""
import contextlib
import http.server
import http.client
import json
import os
from pathlib import Path
import socket
import socketserver
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

REPO = Path(__file__).resolve().parents[1]


def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def execute(args, **kwargs):
    result = subprocess.run(args, capture_output=True, text=True, **kwargs)
    if result.returncode:
        raise RuntimeError(f'{args}: {result.stdout} {result.stderr}')
    return result.stdout


def main():
    print(execute(['node', str(REPO/'tests/i18n-browser.cjs')]))
    print(execute(['node', str(REPO/'tests/pagination.cjs')]))
    with tempfile.TemporaryDirectory(prefix='veydeck-websites-test-') as temporary:
        root = Path(temporary)
        root.chmod(0o755)
        instance = str(uuid.uuid4())
        container = 'veydeck-websites-test-' + uuid.uuid4().hex[:8]
        runtime_token = uuid.uuid4().hex
        entry_token = uuid.uuid4().hex
        api_port, site_port = port(), port()
        base = f'http://127.0.0.1:{api_port}/api/'
        prefix = root / 'instances' / instance
        (root / 'token').write_text(runtime_token)
        (root / 'entry-token').write_text(entry_token)
        (root / 'runtime.json').write_text(json.dumps({'schemaVersion': 1, 'platformVersion': '0.1.0-alpha.1', 'suiteId': 'veydeck.websites', 'instanceId': instance, 'endpoint': {'kind': 'unix', 'socketPath': str(root/'agent.sock'), 'baseUrl': 'http://localhost'}, 'credential': {'tokenPath': str(root/'token')}, 'capabilities': ['services.control', 'operation-logs.write']}))

        failures = {'reload': False}
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
                assert self.headers.get('Authorization') == 'Bearer ' + runtime_token
                try:
                    if self.path.endswith('/operation-events'):
                        result = {'acceptedEventIds': [body['eventId']]}
                    else:
                        action = self.path.rsplit('/', 1)[-1]
                        if action == 'reload' and failures['reload']:
                            failures['reload'] = False
                            raise RuntimeError('injected reload failure')
                        if action == 'validate':
                            revision = str(uuid.UUID(body['revision']))
                            args = ['-t', '-c', str(prefix/'releases'/revision/'nginx.conf')]
                        elif action == 'status':
                            args = ['-v']
                        elif action in ('reload', 'reopen'):
                            args = ['-s', action, '-c', str(prefix/'current/nginx.conf')]
                        else:
                            raise ValueError('unsupported action')
                        result = {'output': execute(['docker', 'exec', container, '/usr/sbin/nginx', *args])}
                    self.send_response(200)
                    output = {'success': True, 'data': result}
                except Exception as error:
                    self.send_response(400)
                    output = {'error': str(error) + ' ' + getattr(error,'stderr','')}
                data = json.dumps(output).encode()
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                with contextlib.suppress(BrokenPipeError): self.wfile.write(data)

            def log_message(self, *_):
                pass

        class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
            daemon_threads = True

        server = Server(str(root/'agent.sock'), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        env = {**os.environ, 'VEYDECK_WEBSITE_DATA_DIR': str(root), 'VEYDECK_SUITE_INSTANCE_ID': instance, 'VEYDECK_SUITE_ID': 'veydeck.websites', 'VEYDECK_ENTRY_TOKEN_FILE': str(root/'entry-token'), 'VEYDECK_AGENT_RUNTIME': str(root/'runtime.json'), 'VEYDECK_WEBSITE_LISTEN': f'127.0.0.1:{api_port}', 'RUST_LOG': 'info'}
        log = (root/'manager.log').open('w')
        process = subprocess.Popen([str(REPO/'target/debug/veydeck-website-manager')], cwd=REPO, env=env, stdout=log, stderr=log)

        def api(path, method='GET', data=None, expected=200):
            headers = {'x-veydeck-entry-token': entry_token}
            if isinstance(data, bytes):
                body = data
            elif data is not None:
                body = json.dumps(data).encode()
                headers['Content-Type'] = 'application/json'
            else:
                body = None
            request = urllib.request.Request(base+path, data=body, headers=headers, method=method)
            try:
                response = urllib.request.urlopen(request, timeout=15)
            except urllib.error.HTTPError as error:
                response = error
            value = response.read()
            assert response.status == expected, (path, response.status, expected, value)
            if not value: return None
            try:
                result = json.loads(value)
                if expected in (400, 409):
                    assert isinstance(result.get('code'), str) and isinstance(result.get('params'), dict), result
                return result
            except json.JSONDecodeError: return value.decode()

        def visit(path='/', expected=200):
            request = urllib.request.Request(f'http://127.0.0.1:{site_port}{path}', headers={'Host': 'test.example'})
            try:
                response = urllib.request.urlopen(request, timeout=5)
            except urllib.error.HTTPError as error:
                response = error
            assert response.status == expected, (path, response.status, response.read())
            return response.read().decode()

        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise AssertionError((root/'manager.log').read_text())
                try:
                    api('state')
                    break
                except OSError:
                    time.sleep(.1)
            else:
                raise AssertionError('manager readiness timeout')
            authorities=api('certificate-authorities')
            assert len(authorities)==1 and authorities[0]['isDefault']
            default_ca=authorities[0]
            assert default_ca['notAfter']==253402300799000
            assert default_ca['publicKey']=='EC 256 bit' and not any('private' in k.lower() for k in default_ca)
            # 管理凭据必需，不能从共享网络直接绕过主控。
            try:
                urllib.request.urlopen(base+'state')
                raise AssertionError('unauthorized API was accepted')
            except urllib.error.HTTPError as error:
                assert error.code == 401
            execute(['docker', 'run', '-d', '--name', container, '--network', 'host', '--ulimit', 'nofile=65536:65536', '--read-only', '--tmpfs', '/tmp', '--tmpfs', '/var/run/nginx', '-v', f'{root}:{root}:ro', '-v', f'{prefix}/logs:{prefix}/logs', 'nginx:1.30.6-trixie@sha256:a7675fcee220f59b4a99f7098c06a0d5d999c2c0ee37c0211efb780536e5cb20', '/usr/sbin/nginx', '-e', 'stderr', '-c', str(prefix/'current/nginx.conf'), '-g', 'daemon off;'])
            time.sleep(.5)
            site = api('sites', 'POST', {'id':'', 'revision':0,'name':'集成网站','enabled':True,'domains':['test.example'],'listenIp':'127.0.0.1','port':site_port,'kind':'static','runDir':'.','spa':False,'upstream':'http://127.0.0.1:3000','certificateId':None,'advanced':''})
            sid = site['id']
            directory = root/'sites/test.example/index'
            assert (directory/'index.html').exists() and (directory/'404.html').exists()
            assert '网站已就绪' in visit()
            visit('/missing',404)
            (directory/'index.html').write_text('custom')
            assert visit() == 'custom'
            (directory/'dist').mkdir(); (directory/'dist/index.html').write_text('dist version')
            site['runDir']='dist';site=api('sites/'+sid,'PUT',site)
            assert visit()=='dist version'
            stale=dict(site);site=api('sites/'+sid,'PUT',site)
            api('sites/'+sid,'PUT',stale,409)
            # 代号唯一，域名在相同端口内唯一；不同端口可复用域名。
            duplicate={**site,'directory':'duplicate-code','domains':['TEST.EXAMPLE.'],'enabled':False}
            api('sites','POST',duplicate,400)
            api('sites','POST',{**site,'directory':'duplicate-code','domains':[]},400)
            for domain in ['../escape','x/y','example.com:80','a..b','-bad.example','user@example.com']:
                api('sites','POST',{**site,'directory':'invalid-domain','domains':[domain]},400)
            for code in ['../escape','/absolute','.hidden','a/b','bad name','a;return']:
                api('sites','POST',{**site,'directory':code,'domains':['code.example']},400)
            other_port=port()
            extra=api('sites','POST',{**site,'directory':'custom_code-2','port':other_port,'domains':['TEST.EXAMPLE.'],'listenIp':'127.0.0.2'})
            assert extra['directory']=='custom_code-2' and extra['listenIp']=='0.0.0.0'
            assert (root/'sites/custom_code-2/index/index.html').exists()
            api('sites/'+extra['id'],'PUT',{**extra,'directory':'changed'},400)
            api('sites','POST',{**site,'port':port(),'domains':['other.example']},400)
            api('sites/'+extra['id']+'?deleteFiles=true','DELETE',expected=204)
            # 两个无域名网站分别按端口提供服务，任意 Host 均可访问。
            catchalls=[]
            for number in range(2):
                listener_port=port()
                fallback=api('sites','POST',{**site,'directory':f'local_proxy_{number}','port':listener_port,'domains':['0.0.0.0'],'kind':'proxy','upstream':f'http://127.0.0.1:{site_port}'})
                # 反代保留 Host，上游显式使用 test.example 验证端口路由。
                req=urllib.request.Request(f'http://127.0.0.1:{listener_port}/',headers={'Host':'test.example'})
                with urllib.request.urlopen(req) as response: assert response.read()==b'dist version'
                fallback['kind']='static';fallback=api('sites/'+fallback['id'],'PUT',fallback)
                req=urllib.request.Request(f'http://127.0.0.1:{listener_port}/',headers={'Host':'unconfigured.local'})
                with urllib.request.urlopen(req) as response: assert '网站已就绪'.encode() in response.read()
                catchalls.append(fallback)
            for fallback in catchalls:api('sites/'+fallback['id']+'?deleteFiles=true','DELETE',expected=204)
            # IPv6 默认关闭；同端口各站点开关互不影响。
            def visit6(host='test.example',target_port=site_port):
                connection=http.client.HTTPConnection('::1',target_port,timeout=3)
                try:
                    connection.request('GET','/',headers={'Host':host})
                    response=connection.getresponse()
                    assert response.status==200
                    return response.read()
                finally: connection.close()
            assert site['ipv6Enabled'] is False
            shared=api('sites','POST',{**site,'directory':'ipv6_shared','domains':['ipv6.example'],'ipv6Enabled':True})
            assert '网站已就绪'.encode() in visit6('ipv6.example')
            try:visit6()
            except http.client.RemoteDisconnected:pass
            else:raise AssertionError('IPv4-only site exposed via shared IPv6 socket')
            site['ipv6Enabled']=True;site=api('sites/'+sid,'PUT',site)
            assert visit6()==b'dist version'
            # fallback + 命名 vhost 共享 IPv6 socket，ipv6only 只能声明一次。
            shared['domains']=['0.0.0.0'];shared=api('sites/'+shared['id'],'PUT',shared)
            assert '网站已就绪'.encode() in visit6('unmatched.local')
            site['ipv6Enabled']=False;site=api('sites/'+sid,'PUT',site)
            assert '网站已就绪'.encode() in visit6()
            assert visit()=='dist version'
            api('sites/'+shared['id']+'?deleteFiles=true','DELETE',expected=204)
            for _ in range(40):
                try:
                    with socket.create_connection(('::1',site_port),timeout=.2):pass
                except ConnectionRefusedError:break
                time.sleep(.1)
            else:raise AssertionError('IPv6 socket remained open after disabling last site')
            site['ipv6Enabled']=True;site=api('sites/'+sid,'PUT',site)
            assert visit6()==b'dist version' and visit()=='dist version'
            # 失败保存不会改变数据库或当前运行目录。
            before=api('state')['sites']
            api('sites/'+sid,'PUT',{**site,'advanced':'not_a_directive on;'},400)
            assert api('state')['sites']==before and visit()=='dist version'
            api('sites/'+sid,'PUT',{**site,'runDir':'../escape'},400)
            (directory/'link').symlink_to('/tmp',target_is_directory=True)
            api('sites/'+sid,'PUT',{**site,'runDir':'link'},400)
            # 已移除的管理接口与网站字段不能继续生效。
            api('policy','PUT',{'mode':'block','rules':[]},404)
            api('waf/templates',expected=404)
            api('logs?kind=waf&site='+sid,expected=400)
            api('sites/'+sid,'PUT',{**site,'wafEnabled':True},422)
            assert 'policy' not in api('state')
            assert not any(key.startswith('waf') for key in site)
            # 全局和站点配置、日志独立保存。
            active=prefix/'current'
            assert 'server_name test.example' not in (active/'nginx.conf').read_text()
            assert 'server_name test.example' in (active/'conf.d/test.example.conf').read_text()
            assert (prefix/'logs/sites/test.example/access.log').exists()
            assert (prefix/'logs/nginx/error.log').exists()
            # 全局性能修改须真正加载，并与网站保存、失败恢复和重启保持一致。
            original_performance = api('settings/performance')
            performance = api('settings/performance', 'PUT', {**original_performance,
                'serverNamesHashBucketSize': 512, 'clientHeaderBufferSizeKb': 32,
                'clientMaxBodySizeMb': 1, 'keepaliveTimeout': 60,
                'gzip': True, 'gzipMinLength': 1024, 'gzipCompLevel': 2})
            assert api('state')['performance'] == performance
            (directory/'dist/compressed.html').write_text('compression test ' * 2000)
            request = urllib.request.Request(f'http://127.0.0.1:{site_port}/compressed.html', headers={'Host':'test.example','Accept-Encoding':'gzip'})
            with urllib.request.urlopen(request) as response:
                assert response.headers.get('Content-Encoding') == 'gzip'
            oversized = urllib.request.Request(f'http://127.0.0.1:{site_port}/', data=b'x'*(1024*1024+1), headers={'Host':'test.example'})
            try: urllib.request.urlopen(oversized)
            except urllib.error.HTTPError as error: assert error.code == 413
            else: raise AssertionError('body size was not enforced')
            api('settings/performance', 'PUT', original_performance, 409)
            api('settings/performance', 'PUT', {**performance, 'gzipCompLevel': 10}, 400)
            api('settings/performance', 'PUT', {**performance, 'serverNamesHashBucketSize': 33}, 400)
            failures['reload'] = True
            api('settings/performance', 'PUT', {**performance, 'gzip': False}, 400)
            assert api('settings/performance') == performance
            with urllib.request.urlopen(request) as response:
                assert response.headers.get('Content-Encoding') == 'gzip'
            site = api('sites/'+sid, 'PUT', site)
            # 网站级非法配置使 nginx -t 失败时，数据库和全局配置都保持原值。
            api('sites/'+sid, 'PUT', {**site, 'advanced':'invalid_global_test on;'}, 400)
            assert api('settings/performance') == performance
            assert 'gzip on;' in (prefix/'current/nginx.conf').read_text()
            print('PASS: global performance runtime gzip/body limit, invalid/stale writes, site save retention')
            # 上传有效证书，在真实 SSL listener 上访问。
            execute(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(root/'key.pem'),'-out',str(root/'cert.pem'),'-days','2','-subj','/CN=test.example/O=Veydeck Test CA','-addext','subjectAltName=DNS:test.example,DNS:*.test.example,IP:192.0.2.1,IP:2001:db8::1'])
            cert=api('certificates','POST',{'certificate':(root/'cert.pem').read_text(),'key':(root/'key.pem').read_text()})
            assert 'name' not in cert and cert['domain']=='test.example'
            assert cert['domains']==['test.example','*.test.example','192.0.2.1','2001:db8::1']
            assert cert['issuerOrganization']=='Veydeck Test CA' and cert['publicKey']=='RSA 2048 bit'
            assert isinstance(cert['notAfter'],int) and cert['notBefore']<cert['notAfter']
            assert len(cert['fingerprintSha256'].split(':'))==32 and cert['serialNumber']
            cert_path=prefix/'certificates'/cert['id']
            key_before=(cert_path/'key.pem').read_bytes()
            # 模拟旧版只有自定义名称与 OpenSSL 日期的元数据。
            (cert_path/'metadata.json').write_text(json.dumps({'id':cert['id'],'name':'旧证书名称','notBefore':'old date','notAfter':'old date'}))
            site['certificateId']=cert['id']
            site=api('sites/'+sid,'PUT',site)
            import ssl
            request=urllib.request.Request(f'https://127.0.0.1:{site_port}/',headers={'Host':'test.example'})
            with urllib.request.urlopen(request,context=ssl._create_unverified_context()) as response:
                assert response.read()==b'dist version'
            tls6=http.client.HTTPSConnection('::1',site_port,context=ssl._create_unverified_context(),timeout=3)
            tls6.request('GET','/',headers={'Host':'test.example'})
            assert tls6.getresponse().read()==b'dist version';tls6.close()
            api('certificates/'+cert['id'],'DELETE',expected=400)
            # 默认根直接签发，创建独立根后可按 DNS/IPv4/IPv6 验证真实 TLS。
            default_pem=api('certificate-authorities/'+default_ca['id']+'/download')
            assert default_pem.count('BEGIN CERTIFICATE')==1 and 'PRIVATE KEY' not in default_pem
            (root/'local-root.pem').write_text(default_pem)
            local=api('certificate-authorities/'+default_ca['id']+'/issue','POST',{'domains':['test.example','*.test.example','127.0.0.1','::1'],'algorithm':'ec256'})
            import datetime
            start_year=datetime.datetime.fromtimestamp(local['notBefore']/1000,datetime.timezone.utc).year
            end_year=datetime.datetime.fromtimestamp(local['notAfter']/1000,datetime.timezone.utc).year
            assert end_year-start_year==10
            assert local['chainLength']==2 and local['publicKey']=='EC 256 bit'
            site=api('sites/'+sid,'PUT',{**site,'certificateId':local['id']})
            trusted=ssl.create_default_context(cafile=str(root/'local-root.pem'))
            with urllib.request.urlopen(urllib.request.Request(f'https://127.0.0.1:{site_port}/',headers={'Host':'test.example'}),context=trusted) as response:assert response.read()==b'dist version'
            trusted6=http.client.HTTPSConnection('::1',site_port,context=trusted,timeout=3)
            trusted6.request('GET','/',headers={'Host':'test.example'});assert trusted6.getresponse().read()==b'dist version';trusted6.close()
            local_path=prefix/'certificates'/local['id']/'certificate.pem'
            assert 'OK' in execute(['openssl','verify','-CAfile',str(root/'local-root.pem'),'-purpose','sslserver','-verify_hostname','test.example',str(local_path)])
            api('certificate-authorities/'+default_ca['id']+'/issue','POST',{'domains':['http://invalid.test'],'algorithm':'ec256','years':1},400)
            api('certificate-authorities','POST',{'name':'bad','organization':'','algorithm':'ec256','years':0},400)
            api('certificate-authorities/not-a-uuid/download',expected=400)
            custom_ca=api('certificate-authorities','POST',{'name':'短期测试根','organization':'Test Lab','algorithm':'rsa2048','years':1})
            custom=api('certificate-authorities/'+custom_ca['id']+'/issue','POST',{'domains':['localhost'],'algorithm':'ec256','years':10})
            assert custom['notAfter']==custom_ca['notAfter'] and custom_ca['publicKey']=='RSA 2048 bit'
            site=api('sites/'+sid,'PUT',{**site,'certificateId':cert['id']})
            api('certificates/'+local['id'],'DELETE',expected=204);api('certificates/'+custom['id'],'DELETE',expected=204)
            print('PASS: default/root creation, signed TLS IPv4+IPv6 with actual CA trust and DNS verification, expiry cap, root download excludes private key')
            # HTTP 反向代理与 WebSocket Upgrade 透传。
            upstream_port=port()
            class Upstream(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    if self.headers.get('Upgrade')=='websocket':
                        self.send_response(101);self.send_header('Upgrade','websocket');self.send_header('Connection','Upgrade');self.end_headers()
                    else:
                        self.send_response(200);self.end_headers();self.wfile.write(b'upstream response')
                def log_message(self,*_):pass
            upstream=http.server.ThreadingHTTPServer(('127.0.0.1',upstream_port),Upstream)
            threading.Thread(target=upstream.serve_forever,daemon=True).start()
            try:
                site['certificateId']=None;site['kind']='proxy';site['upstream']=f'http://127.0.0.1:{upstream_port}'
                site=api('sites/'+sid,'PUT',site)
                assert visit()=='upstream response'
                connection=http.client.HTTPConnection('127.0.0.1',site_port,timeout=5)
                connection.request('GET','/socket',headers={'Host':'test.example','Upgrade':'websocket','Connection':'Upgrade'})
                assert connection.getresponse().status==101;connection.close()
            finally:
                upstream.shutdown();upstream.server_close()
            # 一致性归档可读取，数据库完整且不包含运行 socket。
            import io,tarfile,sqlite3
            request=urllib.request.Request(base+'backup',headers={'x-veydeck-entry-token':entry_token})
            with urllib.request.urlopen(request) as response: archive=tarfile.open(fileobj=io.BytesIO(response.read()),mode='r:gz')
            names=archive.getnames();db_name=f'instances/{instance}/state/websites.db'
            ca_key_name=f'instances/{instance}/authorities/{default_ca["id"]}/key.pem'
            assert ca_key_name in names and archive.getmember(ca_key_name).mode&0o777==0o600
            assert db_name in names and not any(name.endswith('.sock') for name in names)
            backup_db=root/'restored.db';backup_db.write_bytes(archive.extractfile(db_name).read())
            with sqlite3.connect(backup_db) as connection: assert connection.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
            # 模拟配置已切换、数据库尚未提交时管理进程中断。
            current=(prefix/'current').resolve()
            pending=str(uuid.uuid4());candidate=prefix/'releases'/pending
            import shutil
            shutil.copytree(current,candidate)
            config=(candidate/'nginx.conf').read_text().replace(current.name,pending)
            (candidate/'nginx.conf').write_text(config)
            with sqlite3.connect(prefix/'state/websites.db') as connection:
                record=json.loads(connection.execute('SELECT document FROM releases WHERE id=?',(current.name,)).fetchone()[0])
                record.update(id=pending,status='switching',previous=current.name)
                connection.execute('INSERT INTO releases(id,document) VALUES(?,?)',(pending,json.dumps(record)))
            process.kill();process.wait(timeout=5)
            (prefix/'current').unlink();(prefix/'current').symlink_to('releases/'+pending)
            execute(['docker','exec',container,'/usr/sbin/nginx','-s','reload','-c',str(prefix/'current/nginx.conf')])
            process=subprocess.Popen([str(REPO/'target/debug/veydeck-website-manager')],cwd=REPO,env=env,stdout=log,stderr=log)
            for _ in range(100):
                if process.poll() is not None:raise AssertionError((root/'manager.log').read_text())
                try:
                    assert api('state')['sites'][0]['id']==sid
                    break
                except OSError:time.sleep(.1)
            else:raise AssertionError('recovery timeout')
            assert api('settings/performance') == performance
            assert 'gzip on;' in (prefix/'current/nginx.conf').read_text()
            assert (prefix/'current').resolve()==current
            migrated=next(c for c in api('state')['certificates'] if c['id']==cert['id'])
            assert migrated==cert and (cert_path/'key.pem').read_bytes()==key_before
            assert next(a for a in api('certificate-authorities') if a['id']==default_ca['id'])['fingerprintSha256']==default_ca['fingerprintSha256']
            print('PASS: default CA preserved after restart; certificate SAN/IPv4/IPv6/issuer/fingerprint/UTC metadata, legacy metadata rebuilt without changing private key or ID')
            # 活跃网站可直接删除，默认保留内容；显式删除清理文件。
            api('sites/'+sid,'DELETE',expected=204)
            assert directory.exists() and not api('state')['sites']
            second=api('sites','POST',{**site,'directory':'second.example','kind':'static','runDir':'.','domains':['second.example'],'certificateId':None})
            second_dir=root/'sites/second.example/index'
            assert second_dir.exists()
            api('sites/'+second['id']+'?deleteFiles=true','DELETE',expected=204)
            assert not second_dir.parent.exists()
            execute(['openssl','req','-x509','-new','-key',str(root/'key.pem'),'-out',str(root/'browser-cert.pem'),'-days','90','-subj','/CN=test.example/O=Veydeck Test CA','-addext','subjectAltName=DNS:test.example,DNS:*.test.example,IP:192.0.2.1,IP:2001:db8::1'])
            print(execute(['node',str(REPO/'tests/browser.cjs')],env={**os.environ,'WEBSITE_BASE':base.removesuffix('api/'),'WEBSITE_ENTRY_TOKEN':entry_token,'WEBSITE_PORT':str(port()),'WEBSITE_CERT_PATH':str(root/'browser-cert.pem'),'WEBSITE_KEY_PATH':str(root/'key.pem')}))
            print('PASS: auth, automatic reload, index directory, domain uniqueness, direct files, run directory, conflict, failed save recovery, removed WAF routes and fields, split configs/logs, TLS, proxy, WebSocket, backup, direct delete')
        except Exception:
            for error_log in (prefix/'logs').rglob('error.log'):
                print(str(error_log), error_log.read_text()[-8000:])
            print((root/'manager.log').read_text())
            if (prefix/'logs/nginx/error.log').exists(): print((prefix/'logs/nginx/error.log').read_text()[-6000:])
            with contextlib.suppress(Exception):
                print(execute(['docker','inspect','--format','{{json .State}}',container]))
                print(subprocess.run(['docker','logs',container],capture_output=True,text=True).stderr)
                print((prefix/'current/nginx.conf').read_text())
            raise
        finally:
            process.terminate()
            with contextlib.suppress(subprocess.TimeoutExpired):
                process.wait(timeout=5)
            if process.poll() is None:
                process.kill()
            subprocess.run(['docker','rm','-f',container],capture_output=True)
            server.shutdown()
            server.server_close()
            log.close()


if __name__ == '__main__':
    main()
