import java.util.Iterator;
import java.util.Map;

class RemoveMatchingKeys {
    int removeKeys(Map<String, Integer> map, String prefix) {
        int removed = 0;
        Iterator<Map.Entry<String, Integer>> it = map.entrySet().iterator();
        while (it.hasNext()) {
            Map.Entry<String, Integer> entry = it.next();
            if (entry.getKey().startsWith(prefix)) {
                it.remove();
                removed++;
            }
        }
        int remaining = map.size();
        System.out.println(remaining);
        return removed;
    }
}
