import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

class GroupByLength {
    Map<Integer, List<String>> group(List<String> words) {
        Map<Integer, List<String>> groups = new HashMap<>();
        int longest = 0;
        for (String word : words) {
            List<String> bucket = groups.get(word.length());
            if (bucket == null) {
                bucket = new ArrayList<>();
                groups.put(word.length(), bucket);
            }
            bucket.add(word);
            longest = Math.max(longest, word.length());
        }
        int distinct = groups.size();
        System.out.println(distinct + " " + longest);
        return groups;
    }
}
