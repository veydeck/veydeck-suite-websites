/** 网站与证书契约。 */
export interface Site {
  id: string;
  revision: number;
  name: string;
  enabled: boolean;
  domains: string[];
  listenIp: string;
  ipv6Enabled: boolean;
  port: number;
  kind: "static" | "proxy";
  runDir: string;
  spa: boolean;
  upstream: string;
  certificateId: string | null;
  directory: string;
  advanced: string;
}
export interface Snapshot {
  sites: Site[];
  certificates: Certificate[];
  hostDataDir: string;
  runtimeConnected: boolean;
}

/** 从 X.509 证书提取的公开信息，有效期为 UTC Unix 毫秒。 */
export interface Certificate {
  id: string;
  domain: string;
  domains: string[];
  issuerOrganization: string;
  subject: string;
  issuer: string;
  serialNumber: string;
  fingerprintSha256: string;
  signatureAlgorithm: string;
  publicKey: string;
  notBefore: number;
  notAfter: number;
  chainLength: number;
}

/** 根证书独立于可供网站选择的服务器证书。 */
export interface CertificateAuthority extends Certificate {
  name: string;
  organization: string;
  isDefault: boolean;
  createdAt: number;
}

/** 全局 Nginx 性能参数；字段后缀注明大小单位。 */
export interface Performance {
  revision: number;
  serverNamesHashBucketSize: number;
  clientHeaderBufferSizeKb: number;
  clientMaxBodySizeMb: number;
  keepaliveTimeout: number;
  gzip: boolean;
  gzipMinLength: number;
  gzipCompLevel: number;
}
