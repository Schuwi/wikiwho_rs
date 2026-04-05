import os

from WikiWho.wikiwho import Wikiwho

# Per-worker state (set by worker_init in each Pool worker process)
_input_file = None
_result_file = None
_result_path = None
_result_offset = 0

def worker_init(input_path, result_dir):
    global _input_file, _result_file, _result_path, _result_offset
    _input_file = open(input_path, 'rb')
    pid = os.getpid()
    _result_path = os.path.join(result_dir, f"result_{pid}.bin")
    _result_file = open(_result_path, 'wb')
    _result_offset = 0

def process_page(page_ref):
    global _result_offset
    offset, length = page_ref

    # Read page bincode directly from shared input file (no GIL contention)
    _input_file.seek(offset)
    page_bincode = _input_file.read(length)

    page = PyPage.from_bincode(page_bincode)
    wikiwho = Wikiwho(f"{page.namespace}:{page.title}")
    wikiwho.analyse_article_from_xml_dump(page)
    key = f"{page.namespace}:{page.title}"
    result_bincode = bytes(serialize_wikiwho_result(wikiwho, page_bincode))

    # Write result to per-worker file (no GIL contention)
    result_offset = _result_offset
    _result_file.write(result_bincode)
    _result_file.flush()
    _result_offset += len(result_bincode)

    return (key, _result_path, result_offset, len(result_bincode))

from multiprocessing import Pool, Process, Queue

def run_analysis_python_mt_impl(work_receiver, result_sender, num_threads, input_path, result_dir):
    processed = 0

    with Pool(num_threads, initializer=worker_init, initargs=(input_path, result_dir)) as pool:
        for processed_page in pool.imap_unordered(process_page, iter(work_receiver.get, None)):
            result_sender.put(processed_page)
            processed += 1
    result_sender.put('close')

def run_analysis_python_mt(num_threads, input_path, result_dir):
    work_receiver = Queue()
    result_sender = Queue()
    p = Process(target=run_analysis_python_mt_impl, args=(work_receiver, result_sender, num_threads, input_path, result_dir))
    p.start()

    return {'work_receiver': work_receiver, 'result_sender': result_sender, 'process': p}