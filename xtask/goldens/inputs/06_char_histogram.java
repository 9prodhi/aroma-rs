import java.util.HashMap;
import java.util.Map;

class CharHistogram {
    Map<Character, Integer> histogram(String text) {
        Map<Character, Integer> counts = new HashMap<>();
        char[] chars = text.toCharArray();
        for (char c : chars) {
            Integer old = counts.get(c);
            if (old == null) {
                counts.put(c, 1);
            } else {
                counts.put(c, old + 1);
            }
        }
        int distinct = counts.size();
        System.out.println(distinct);
        return counts;
    }
}
