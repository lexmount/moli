import assert from 'node:assert/strict';
import { openSync, readSync, closeSync, statSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

export async function checkBlobDownloadDelivery(browser, page, tempDir, fixture) {
  const results = [];
  for (const large of [false, true]) {
    const name = large ? 'large-blob.bin' : 'blob-payload.bin';
    const size = large ? 65 * 1024 * 1024 : 5;
    // Start saveAs directly in the download event callback.
    const saved = page.waitForEvent('download', { timeout: 10_000 }).then(async download => {
      const savedPath = join(tempDir, name);
      await download.saveAs(savedPath);
      assert.equal(download.suggestedFilename(), name);
      assert.equal(await download.failure(), null);
      assert.equal(statSync(savedPath).size, size);
      if (large) {
        const file = openSync(savedPath, 'r');
        try {
          const buffer = Buffer.alloc(64 * 1024);
          for (;;) {
            const read = readSync(file, buffer);
            if (!read) break;
            assert(buffer.subarray(0, read).every(byte => byte === 0xa5));
          }
        } finally {
          closeSync(file);
        }
      } else {
        assert.equal(readFileSync(savedPath).toString('hex'), '00ff80410a');
      }
    });
    await page.evaluate(({ large, size, name }) => {
      const bytes = large ? new Uint8Array(size).fill(0xa5) : new Uint8Array([0, 255, 128, 65, 10]);
      const url = URL.createObjectURL(new Blob([bytes], { type: 'application/octet-stream' }));
      const link = document.createElement('a');
      link.href = url;
      link.download = name;
      document.body.append(link);
      link.click();
      URL.revokeObjectURL(url);
      link.remove();
    }, { large, size, name });
    await saved;
    // Also verifies that a large owner action leaves the output channel usable.
    assert.equal(await page.evaluate(() => 21 * 2), 42);
    results.push(large ? 'large_blob_download_save_as' : 'blob_download_save_as_after_url_revocation');
  }

  const canceled = page.waitForEvent('download', { timeout: 10_000 }).then(async download => {
    assert.equal(download.suggestedFilename(), 'revoked.bin');
    assert.equal(await download.failure(), 'canceled');
    await assert.rejects(download.saveAs(join(tempDir, 'revoked.bin')));
  });
  await page.evaluate(() => {
    const url = URL.createObjectURL(new Blob(['already revoked']));
    const link = document.createElement('a');
    link.href = url;
    link.download = 'revoked.bin';
    document.body.append(link);
    URL.revokeObjectURL(url);
    link.click();
    link.remove();
  });
  await canceled;
  results.push('revoked_blob_download_reports_canceled');

  // The same origin in another browser context has a different storage partition.
  const otherContext = await browser.newContext({ acceptDownloads: true });
  let privateUrl;
  try {
    const otherPage = await otherContext.newPage();
    await otherPage.goto(`${fixture}/download-page`, { waitUntil: 'load', timeout: 10_000 });
    privateUrl = await page.evaluate(() => URL.createObjectURL(new Blob(['private payload'])));
    const denied = otherPage.waitForEvent('download', { timeout: 10_000 }).then(async download => {
      assert.equal(await download.failure(), 'canceled');
      await assert.rejects(download.saveAs(join(tempDir, 'foreign.bin')));
    });
    await otherPage.evaluate(url => {
      const link = document.createElement('a');
      link.href = url;
      link.download = 'foreign.bin';
      document.body.append(link);
      link.click();
      link.remove();
    }, privateUrl);
    await denied;
    results.push('blob_download_storage_partition_isolation');
  } finally {
    if (privateUrl) await page.evaluate(url => URL.revokeObjectURL(url), privateUrl);
    await otherContext.close();
  }
  return results;
}
