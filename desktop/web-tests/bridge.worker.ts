// Direct checks of the OPFS bridge for cases recipe_box's own tests never reach.
import { wasi } from '@bjorn3/browser_wasi_shim';
import { Bridge, OpfsDir, OpfsFile } from '../src/web/wasi-opfs';

self.onmessage = (ev: MessageEvent) => {
  const { ctrl, data, port } = ev.data;
  const dir = new OpfsDir(new Bridge(ctrl, data, port), '/tmp', [], '/tmp');
  const results: string[] = [];
  const check = (name: string, ok: boolean) => results.push(`${ok ? 'ok  ' : 'FAIL'} ${name}`);
  const W = BigInt(wasi.RIGHTS_FD_WRITE);
  const open = (path: string, oflags = 0) => dir.path_open(0, path, oflags, W, 0n, 0).fd_obj as OpfsFile;
  const text = (f: OpfsFile) => new TextDecoder().decode(f.fd_pread(100, 0n).data);
  const bytes = (s: string) => new TextEncoder().encode(s);

  // a descriptor keeps meaning its own file after the path is reused
  const old = open('reuse.txt', wasi.OFLAGS_CREAT | wasi.OFLAGS_TRUNC);
  old.fd_write(bytes('old'));
  check('unlink an open file', dir.path_unlink_file('reuse.txt') === 0);
  const fresh = open('reuse.txt', wasi.OFLAGS_CREAT | wasi.OFLAGS_EXCL);
  fresh.fd_write(bytes('new'));
  check('a stale descriptor is refused, not pointed at the new file', old.fd_write(bytes('X')).ret === wasi.ERRNO_BADF);
  check('the new file is untouched', text(fresh) === 'new');
  old.fd_close();
  check('closing the stale descriptor leaves the new one open', text(fresh) === 'new');
  fresh.fd_close();

  // rename replaces the target in one step, and descriptors on the old target are refused
  const a = open('a.json', wasi.OFLAGS_CREAT | wasi.OFLAGS_TRUNC);
  a.fd_write(bytes('A'));
  a.fd_close();
  const b = open('b.json', wasi.OFLAGS_CREAT | wasi.OFLAGS_TRUNC);
  b.fd_write(bytes('B'));
  const rename = (from: string, to: string) => {
    const x = dir.locate(from), y = dir.locate(to);
    return dir.bridge.call({ op: 'rename', mount: x.mount, parts: x.parts, toMount: y.mount, toParts: y.parts }).reply.err ?? 'ok';
  };
  check('rename over an existing file', rename('a.json', 'b.json') === 'ok');
  check('the old target descriptor is refused', b.fd_write(bytes('Z')).ret === wasi.ERRNO_BADF);
  b.fd_close();
  const after = open('b.json');
  check('the target holds the renamed file whole', text(after) === 'A');
  after.fd_close();
  check('the old name is gone', dir.path_filestat_get(0, 'a.json').ret === wasi.ERRNO_NOENT);
  check('no climbing out of the mount', dir.path_open(0, '../x', wasi.OFLAGS_CREAT, W, 0n, 0).ret === wasi.ERRNO_NOTCAPABLE);

  (self as unknown as Worker).postMessage({ type: 'done', results });
};
