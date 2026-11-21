import java.util.HashMap;
import java.util.Map;

class WordCountMap {
    Map<String, Integer> countWords(String text) {
        Map<String, Integer> counts = new HashMap<>();
        String[] words = text.split(" ");
        for (String word : words) {
            Integer old = counts.get(word);
            if (old == null) {
                counts.put(word, 1);
            } else {
                counts.put(word, old + 1);
            }
        }
        int distinct = counts.size();
        System.out.println(distinct);
        return counts;
    }
}
