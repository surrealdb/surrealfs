"""Tests for Phase 7 understanding pipeline, multilingual search,
and code intelligence.
"""

from __future__ import annotations


async def test_file_jobs_triggered_on_write(fs):
    """Writing a file automatically enqueues pipeline jobs via evt_file_jobs."""
    entry = await fs.write_text("/src/algo.py", "def compute(): pass\n")
    assert entry.path == "/src/algo.py"

    # Query sfs_job table
    res = await fs.db.query("SELECT * FROM sfs_job ORDER BY created_at ASC;")
    jobs = res if isinstance(res, list) else res.get("result", [])
    if isinstance(jobs, list) and len(jobs) > 0 and isinstance(jobs[0], list):
        jobs = jobs[0]

    kinds = [j["kind"] for j in jobs]
    assert "detect" in kinds
    assert "chunk" in kinds
    assert "symbols" in kinds
    for j in jobs:
        assert j["status"] == "pending"
        assert j["attempts"] == 0


async def test_job_claim_and_complete(fs):
    """fn::sfs_job_claim leases pending jobs, fn::sfs_job_complete completes."""
    await fs.write_text("/data/sample.txt", "Some test content for pipeline")

    # Claim jobs using SurrealQL function
    res = await fs.db.query(
        "RETURN fn::sfs_job_claim($worker, $kinds, $limit);",
        {"worker": "worker-py-1", "kinds": ["detect", "chunk"], "limit": 2},
    )
    claimed = res[0] if isinstance(res, list) else res
    if isinstance(claimed, list) and len(claimed) > 0 and isinstance(claimed[0], list):
        claimed = claimed[0]

    assert len(claimed) == 2
    for job in claimed:
        assert job["status"] == "processing"

    # Complete the first job
    job_id = str(claimed[0]["id"])
    await fs.db.query(
        "RETURN fn::sfs_job_complete($job_id, NONE);",
        {"job_id": job_id},
    )

    # Verify status is completed
    res = await fs.db.query(f"SELECT status FROM {job_id};")
    job_record = res[0] if isinstance(res, list) else res
    if isinstance(job_record, list) and len(job_record) > 0:
        job_record = job_record[0]
    assert job_record["status"] == "completed"


async def test_multilingual_indexes_and_analyzers(fs):
    """Multilingual analyzers stem correctly in German, French, Spanish, and CJK."""
    # 1. German
    await fs.write_text("/de.txt", "Die Konfigurationen der Systeme sind wichtig.")
    await fs.db.query("UPDATE file SET language = 'de' WHERE path = '/de.txt';")
    res_de = await fs.db.query(
        "SELECT path, content FROM file WHERE text_de @@ 'Konfiguration';"
    )
    rows_de = res_de[0] if isinstance(res_de, list) else res_de
    assert len(rows_de) >= 1
    assert "Konfigurationen" in rows_de[0]["content"]

    # 2. French
    await fs.write_text("/fr.txt", "Nous mangeons des pommes ensemble.")
    await fs.db.query("UPDATE file SET language = 'fr' WHERE path = '/fr.txt';")
    res_fr = await fs.db.query(
        "SELECT path, content FROM file WHERE text_fr @@ 'pomme';"
    )
    rows_fr = res_fr[0] if isinstance(res_fr, list) else res_fr
    assert len(rows_fr) >= 1

    # 3. Spanish
    await fs.write_text("/es.txt", "Los usuarios corrieron hacia la salida.")
    await fs.db.query("UPDATE file SET language = 'es' WHERE path = '/es.txt';")
    res_es = await fs.db.query(
        "SELECT path, content FROM file WHERE text_es @@ 'correr';"
    )
    rows_es = res_es[0] if isinstance(res_es, list) else res_es
    assert len(rows_es) >= 1

    # 4. CJK (ngram)
    await fs.write_text("/cjk.txt", "数据库系统设计与实现")
    await fs.db.query("UPDATE file SET language = 'zh' WHERE path = '/cjk.txt';")
    res_cjk = await fs.db.query(
        "SELECT path, content FROM file WHERE text_cjk @@ '系统';"
    )
    rows_cjk = res_cjk[0] if isinstance(res_cjk, list) else res_cjk
    assert len(rows_cjk) >= 1

    # 5. Code
    await fs.write_text("/code.txt", "class AsyncConnectionPool extends BasePool {}")
    await fs.db.query("UPDATE file SET language = 'code' WHERE path = '/code.txt';")
    res_code = await fs.db.query(
        "SELECT path, content FROM file WHERE text_code @@ 'connection';"
    )
    rows_code = res_code[0] if isinstance(res_code, list) else res_code
    assert len(rows_code) >= 1


async def test_symbols_and_definition_surql_functions(fs):
    """fn::sfs_symbols and fn::sfs_definition retrieve indexed symbols."""
    # Insert file and symbols
    await fs.write_text("/src/app.py", "def start_app(): pass\n")
    res_f = await fs.db.query("RETURN fn::sfs_resolve('/src/app.py');")
    file_id = res_f[0]

    await fs.db.query(
        """
        CREATE symbol CONTENT {
            file_id: $fid,
            name: 'start_app',
            qualified: 'app.start_app',
            kind: 'function',
            language: 'python',
            signature: 'def start_app()',
            doc: 'Entrypoint function',
            line_start: 1,
            line_end: 2
        };
        """,
        {"fid": file_id},
    )

    # 1. Query symbols by path
    res_sym = await fs.db.query("RETURN fn::sfs_symbols('/src/app.py', 'root');")
    symbols = res_sym[0]
    assert len(symbols) == 1
    assert symbols[0]["name"] == "start_app"
    assert symbols[0]["kind"] == "function"

    # 2. Query definition by name
    res_def = await fs.db.query("RETURN fn::sfs_definition('start_app', 'root');")
    defs = res_def[0]
    assert len(defs) == 1
    assert defs[0]["name"] == "start_app"
    assert defs[0]["doc"] == "Entrypoint function"


async def test_folder_digest_and_packing(fs):
    """fn::sfs_digest summaries folders and fn::sfs_pack packs relevant context."""
    await fs.write_text(
        "/docs/guide.md", "# Guide\nHow to configure the application.\n"
    )
    await fs.write_text("/docs/api.md", "# API\nEndpoint definitions and schemas.\n")

    # 1. Digest
    res_dig = await fs.db.query("RETURN fn::sfs_digest('/docs', 'root');")
    digest = res_dig[0]
    assert digest["path"] == "/docs"
    assert digest["total_readable"] == 2
    assert len(digest["notable_files"]) == 2
    assert digest["private_unsummarised"] == 0

    # 2. Pack
    res_pack = await fs.db.query(
        "RETURN fn::sfs_pack('configuration endpoints', 1000, '/docs', 'root');"
    )
    pack = res_pack[0]
    assert pack["question"] == "configuration endpoints"
    assert pack["budget"] == 1000
    assert len(pack["candidates"]) >= 1
