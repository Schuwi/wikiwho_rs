import collections
from types import FunctionType, SimpleNamespace

def imprint(obj):
    """
    Recursively convert an object into a SimpleNamespace with pickleable attributes.
    
    Args:
    obj: The original object to imprint.
    
    Returns:
    A SimpleNamespace representing the imprinted object.
    """
    # Base cases: immutable and already pickleable types
    if isinstance(obj, (int, float, str, bool, type(None))):
        return obj
    
    # Handle lists, tuples, and sets by processing each element
    elif isinstance(obj, list):
        return [imprint(item) for item in obj]
    elif isinstance(obj, tuple):
        return tuple(imprint(item) for item in obj)
    elif isinstance(obj, set):
        return {imprint(item) for item in obj}
    
    # Handle Python functions and types by returning them as-is
    elif isinstance(obj, (type, FunctionType)):
        return obj
    
    # Handle dictionaries by processing keys and values
    elif isinstance(obj, dict):
        return {imprint(key): imprint(value) for key, value in obj.items()}
    
    # Handle objects with __dict__ (most user-defined classes)
    elif hasattr(obj, '__dict__'):
        # Process each attribute
        processed_attrs = {}
        for attr, value in vars(obj).items():
            processed_attrs[attr] = imprint(value)
        return SimpleNamespace(**processed_attrs)
    
    # Handle objects that are iterable but don't have __dict__
    elif isinstance(obj, collections.abc.Iterable):
        try:
            return type(obj)(imprint(item) for item in obj)
        except TypeError as err:
            raise err # Not all iterables can be handled generically
    
    # Fallback: represent the object as its string representation
    else:
        return repr(obj)



from WikiWho.wikiwho import Wikiwho
import pickle

def process_page(page):
    wikiwho = Wikiwho(f"{page.namespace}:{page.title}") # Use the title for identification in multi-threaded processing
    wikiwho.analyse_article_from_xml_dump(page)
    return pickle.dumps(wikiwho, protocol=5)

from multiprocessing import Pool, Process, Queue

def run_analysis_python_mt_impl(work_receiver, result_sender, num_threads):
    processed = 0

    with Pool(num_threads) as pool:
        for processed_page in pool.imap_unordered(process_page, iter(work_receiver.get, None)):
            result_sender.put(processed_page)
            processed += 1
    result_sender.put('close')

def run_analysis_python_mt(num_threads):
    work_receiver = Queue()
    result_sender = Queue()
    p = Process(target=run_analysis_python_mt_impl, args=(work_receiver, result_sender, num_threads))
    p.start()

    return {'work_receiver': work_receiver, 'result_sender': result_sender, 'process': p}