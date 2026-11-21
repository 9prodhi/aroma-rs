class CountVowels {
    int countVowels(String text) {
        String lower = text.toLowerCase();
        int count = 0;
        int others = 0;
        for (int i = 0; i < lower.length(); i++) {
            char c = lower.charAt(i);
            if ("aeiou".indexOf(c) >= 0) {
                count++;
            } else {
                others++;
            }
        }
        System.out.println(others);
        return count;
    }
}
